#!/usr/bin/env python3
"""Calls the proxy of a running session.

Usage:
  drive.py SESSION_DIR TOOL ['JSON_ARGS']
  drive.py SESSION_DIR --list | --stop | --mark NAME
"""
import json
import socket
import sys


def call(session_dir, request):
    with socket.socket(socket.AF_UNIX) as sock:
        sock.connect(f"{session_dir}/proxy.sock")
        sock.sendall((json.dumps(request) + "\n").encode())
        data = b""
        while not data.endswith(b"\n"):
            chunk = sock.recv(1 << 20)
            if not chunk:
                break
            data += chunk
    return json.loads(data)


def main():
    session_dir, command, *rest = sys.argv[1:]
    if command == "--list":
        request = {"list": True}
    elif command == "--stop":
        request = {"stop": True}
    elif command == "--mark":
        request = {"mark": rest[0]}
    else:
        request = {"tool": command, "args": json.loads(rest[0]) if rest else {}}
    print(json.dumps(call(session_dir, request), indent=1))


if __name__ == "__main__":
    main()
