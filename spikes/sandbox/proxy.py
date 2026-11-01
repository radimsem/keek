#!/usr/bin/env python3
"""Starts one sandbox and forwards tool calls to its Cua Driver, timing each.

Usage: proxy.py SESSION_DIR
Control socket: SESSION_DIR/proxy.sock, one JSON request per connection:
  {"tool": NAME, "args": {...}}   forward a tool call
  {"tool": "start_recording", ...} also reads the Session Timeline zero
  {"mark": NAME}                  log a named time, such as a Takeover edge
  {"list": true}                  list tools
  {"stop": true}                  stop recording and shut the sandbox down
Times in calls.jsonl are milliseconds of CLOCK_MONOTONIC after the zero.
"""
import base64
import ctypes
import json
import os
import pathlib
import signal
import socket
import subprocess
import sys
import time

HERE = pathlib.Path(__file__).resolve().parent
SESSION = pathlib.Path(sys.argv[1]).resolve()
PR_SET_PDEATHSIG = 1


def die_with_proxy():
    ctypes.CDLL(None).prctl(PR_SET_PDEATHSIG, signal.SIGKILL)


class Driver:
    def __init__(self):
        self.process = subprocess.Popen(
            [str(HERE / "sandbox.sh"), str(SESSION)],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=(SESSION / "sandbox.err").open("wb"),
            preexec_fn=die_with_proxy,
        )
        self.next_id = 0

    def send(self, message):
        self.process.stdin.write((json.dumps(message) + "\n").encode())
        self.process.stdin.flush()

    def request(self, method, params):
        self.next_id += 1
        self.send({"jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params})
        while True:
            line = self.process.stdout.readline()
            if not line:
                raise RuntimeError(f"The driver closed stdout. See {SESSION}/driver.log.")
            message = json.loads(line)
            if message.get("id") == self.next_id and "method" not in message:
                return message
            if "method" in message and "id" in message:
                self.send({"jsonrpc": "2.0", "id": message["id"], "result": {}})


def save_images(result, counter):
    """Writes images to the session folder, so a Verifier can read them by path."""
    content = result.get("result", {}).get("content", [])
    for item in content:
        if item.get("type") == "image":
            suffix = ".jpg" if "jpeg" in item.get("mimeType", "") else ".png"
            path = SESSION / "shots" / f"{counter:05}{suffix}"
            path.parent.mkdir(exist_ok=True)
            path.write_bytes(base64.b64decode(item["data"]))
            item.pop("data")
            item["path"] = str(path)
    return result


def main():
    driver = Driver()
    init = driver.request(
        "initialize",
        {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "keek-spike-proxy", "version": "0"},
        },
    )
    driver.send({"jsonrpc": "2.0", "method": "notifications/initialized"})
    tools = driver.request("tools/list", {})["result"]["tools"]
    read_only = {t["name"] for t in tools if t.get("annotations", {}).get("readOnlyHint")}
    (SESSION / "init.json").write_text(json.dumps(init, indent=2))

    server = socket.socket(socket.AF_UNIX)
    sock_path = SESSION / "proxy.sock"
    sock_path.unlink(missing_ok=True)
    server.bind(str(sock_path))
    server.listen()

    zero_ns = None
    calls = (SESSION / "calls.jsonl").open("a", buffering=1)
    counter = 0

    def since_zero(ns):
        return None if zero_ns is None else (ns - zero_ns) / 1e6

    running = True
    while running:
        conn, _ = server.accept()
        with conn:
            data = b""
            while not data.endswith(b"\n"):
                chunk = conn.recv(1 << 20)
                if not chunk:
                    break
                data += chunk
            request = json.loads(data)
            if "list" in request:
                reply = {"tools": [t["name"] for t in tools], "read_only": sorted(read_only)}
            elif "mark" in request:
                now = time.monotonic_ns()
                entry = {"mark": request["mark"], "t": since_zero(now)}
                calls.write(json.dumps(entry) + "\n")
                reply = entry
            elif "stop" in request:
                reply = {"stopping": True}
                running = False
            else:
                counter += 1
                tool = request["tool"]
                if tool == "start_recording":
                    zero_ns = time.monotonic_ns()
                    zero_wall = time.time_ns()
                    (SESSION / "zero.json").write_text(
                        json.dumps({"monotonic_ns": zero_ns, "realtime_ns": zero_wall})
                    )
                start = time.monotonic_ns()
                result = driver.request(
                    "tools/call", {"name": tool, "arguments": request.get("args", {})}
                )
                end = time.monotonic_ns()
                calls.write(
                    json.dumps(
                        {
                            "tool": tool,
                            "action": tool not in read_only,
                            "start": since_zero(start),
                            "end": since_zero(end),
                        }
                    )
                    + "\n"
                )
                reply = save_images(result, counter)
            conn.sendall((json.dumps(reply) + "\n").encode())

    driver.process.stdin.close()
    try:
        driver.process.wait(timeout=10)
    except subprocess.TimeoutExpired:
        driver.process.kill()
    sock_path.unlink(missing_ok=True)


if __name__ == "__main__":
    os.chdir(SESSION)
    main()
