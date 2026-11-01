#!/usr/bin/env python3
"""Measures how far the proxy's clock sits from the video.

Usage:
  measure.py run SESSION_DIR CLICKS     run one flip session and record it
  measure.py analyze SESSION_DIR...     print the table and its statistics

A session's page flips the whole screen on each click. The difference for a
click is the flip's time in the video minus the proxy's time for the click.
Times are milliseconds. The proxy counts from just before start_recording.
"""
from __future__ import annotations

import json
import os
import pathlib
import random
import re
import statistics
import subprocess
import sys
import time

from drive import call

HERE = pathlib.Path(__file__).resolve().parent
PAGE_LOG = "page-events.jsonl"
FLIP_SCORE = 0.9


def start_session(session: pathlib.Path, page: str) -> subprocess.Popen:
    session.mkdir(parents=True, exist_ok=False)
    env = {**os.environ, "KEEK_PAGE": page}
    with (session / "proxy.out").open("wb") as log:
        proxy = subprocess.Popen(
            [str(HERE / "proxy.py"), str(session)], env=env, stdout=log, stderr=log
        )
    return proxy


def find_window(session: pathlib.Path, timeout: float = 30.0) -> tuple[int, int]:
    """Returns (pid, window_id) of the first Chromium window."""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            reply = call(str(session), {"tool": "list_windows", "args": {}})
        except (FileNotFoundError, ConnectionRefusedError):
            time.sleep(0.2)
            continue
        text = reply["result"]["content"][0]["text"]
        match = re.search(r'window_id=(\d+) pid=(\d+) "[^"]*Chromium"', text)
        if match:
            return int(match.group(2)), int(match.group(1))
        time.sleep(0.2)
    raise RuntimeError(f"No Chromium window appeared. See {session}/chromium.log.")


def tool(session: pathlib.Path, name: str, **args: object) -> dict:
    reply = call(str(session), {"tool": name, "args": args})
    result = reply.get("result", {})
    if result.get("isError") or "error" in reply:
        raise RuntimeError(f"{name} failed: {json.dumps(reply)[:600]}")
    return result


def run(session: pathlib.Path, clicks: int) -> None:
    proxy = start_session(session, "flip")
    try:
        pid, window_id = find_window(session)
        # Chromium paints the page after it maps, and the page must be up before the first click.
        time.sleep(5)
        # The driver refuses pixels until it has seen the window's own screenshot.
        tool(session, "get_window_state", pid=pid, window_id=window_id)
        tool(session, "start_recording", output_dir=str(session / "rec"), record_video=True)
        time.sleep(2)
        for _ in range(clicks):
            tool(
                session,
                "click",
                pid=pid,
                window_id=window_id,
                x=640,
                y=400,
                delivery_mode="foreground",
            )
            # Uneven gaps keep a fixed period from hiding a drift.
            time.sleep(random.uniform(0.7, 1.5))
        tool(session, "stop_recording")
    finally:
        call(str(session), {"stop": True})
        proxy.wait(timeout=20)


def video_scenes(video: pathlib.Path) -> list[tuple[float, float]]:
    """Returns (time in ms, scene score) for each frame past the ticket's threshold."""
    output = subprocess.run(
        [
            "ffmpeg", "-hide_banner", "-i", str(video),
            "-vf", "select='gt(scene,0.3)',showinfo,metadata=print:key=lavfi.scene_score",
            "-f", "null", "-",
        ],
        capture_output=True,
        text=True,
        check=True,
    ).stderr
    # metadata=print writes its own pts_time line directly above each score.
    pairs = re.findall(r"pts_time:([0-9.]+)\s*\n[^\n]*lavfi\.scene_score=([0-9.]+)", output)
    return [(float(t) * 1000, float(score)) for t, score in pairs]


def analyze_session(session: pathlib.Path) -> list[dict]:
    calls = [json.loads(line) for line in (session / "calls.jsonl").read_text().splitlines()]
    clicks = [entry for entry in calls if entry.get("tool") == "click"]
    scenes = video_scenes(session / "rec" / "recording.mp4")
    # A full-screen flip scores 1. Anything lower is the desktop repainting.
    flips = [time_ms for time_ms, score in scenes if score >= FLIP_SCORE]
    others = [(time_ms, score) for time_ms, score in scenes if score < FLIP_SCORE]
    if others:
        print(f"{session.name}: ignored scene changes (ms, score): {others}", file=sys.stderr)
    if len(flips) != len(clicks):
        raise RuntimeError(f"{session.name}: {len(clicks)} clicks but {len(flips)} flips in the video.")

    zero = json.loads((session / "zero.json").read_text())
    zero_ms = zero["realtime_ns"] / 1e6
    page = [
        json.loads(line)
        for line in (session / PAGE_LOG).read_text().splitlines()
    ]
    page = sorted((e for e in page if e.get("type") == "flip"), key=lambda e: e["t"])
    if len(page) != len(clicks):
        raise RuntimeError(f"{session.name}: {len(clicks)} clicks but {len(page)} page events.")

    rows = []
    for index, (click, flip) in enumerate(zip(clicks, flips, strict=True)):
        row = {
            "session": session.name,
            "n": index + 1,
            "start": click["start"],
            "end": click["end"],
            "flip": flip,
            "flip_minus_start": flip - click["start"],
            "flip_minus_end": flip - click["end"],
        }
        row["page_t"] = page[index]["t"] - zero_ms
        row["flip_minus_page"] = flip - row["page_t"]
        rows.append(row)
    return rows


def describe(values: list[float]) -> str:
    return (
        f"min {min(values):8.1f}  median {statistics.median(values):8.1f}  "
        f"max {max(values):8.1f}  spread {max(values) - min(values):7.1f}"
    )


def analyze(sessions: list[pathlib.Path]) -> None:
    rows = [row for session in sessions for row in analyze_session(session)]
    print("session | n | click start | click end | page event | flip in video | flip-start | flip-end | flip-page")
    print("--- | --- | --- | --- | --- | --- | --- | --- | ---")
    for row in rows:
        print(
            f"{row['session']} | {row['n']} | {row['start']:.1f} | {row['end']:.1f} | "
            f"{row['page_t']:.1f} | {row['flip']:.1f} | "
            f"{row['flip_minus_start']:.1f} | {row['flip_minus_end']:.1f} | "
            f"{row['flip_minus_page']:.1f}"
        )
    print()
    print(f"clicks: {len(rows)} across {len(sessions)} sessions")
    print("flip - click start:", describe([r["flip_minus_start"] for r in rows]))
    print("flip - click end:  ", describe([r["flip_minus_end"] for r in rows]))
    with_page = [r["flip_minus_page"] for r in rows]
    print("flip - page event: ", describe(with_page))
    # A flip can't reach the video before the page acts, so the video clock
    # sits at least this far behind the proxy's.
    print(f"C lower bound (-min of flip - page event): {-min(with_page):.1f}")
    for session in sessions:
        session_rows = [r for r in rows if r["session"] == session.name]
        print(f"{session.name} flip - start:", describe([r["flip_minus_start"] for r in session_rows]))


def main() -> None:
    command, *args = sys.argv[1:]
    if command == "run":
        run(pathlib.Path(args[0]).resolve(), int(args[1]))
    elif command == "analyze":
        analyze([pathlib.Path(arg).resolve() for arg in args])
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
