#!/usr/bin/env python3
"""Checks that a Takeover cut made from proxy times leaves no Takeover footage.

Usage:
  takeover_cut.py run SESSION_DIR               run a session with a scripted Takeover
  takeover_cut.py person SESSION_DIR            run a guided Takeover with a person at the desktop
  takeover_cut.py cut SESSION_DIR C_MS GUARD_MS cut the recording and scan what is left

The page shows solid magenta from Take control to Done. Take control reads the
clock and then turns the page magenta. Done reads the clock while the page is
still magenta, which is the late case: an echo of the last key can land after Done.

A cut removes [start - C - GUARD, end - C + GUARD] in video time, where C is how
far the video's clock sits behind the proxy's zero.
"""
from __future__ import annotations

import json
import pathlib
import re
import subprocess
import sys
import time

from drive import call
from measure import find_window, start_session, tool

# Solid magenta under the window's opacity. Neutral chroma sits near 128.
MAGENTA_U = 160
MAGENTA_V = 160
TAKEOVER_LIMIT_S = 180
NO_FOCUS_CHECK_S = 15
CUT_LENGTH_TOLERANCE_S = 0.5


def mark(session: pathlib.Path, name: str) -> float:
    return call(str(session), {"mark": name})["t"]


def set_page(session: pathlib.Path, colour: str) -> None:
    path = session / "page-state"
    temporary = session / "page-state.tmp"
    temporary.write_text(colour)
    temporary.replace(path)


def wait_for_colour(session: pathlib.Path, colour: str, timeout: float = 5.0) -> None:
    log = session / "page-events.jsonl"
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if log.exists() and any(
            json.loads(line).get("colour") == colour for line in log.read_text().splitlines()
        ):
            return
        time.sleep(0.01)
    raise RuntimeError(f"The page never reported {colour}. See {session}/page-server.log.")


def record_around(session: pathlib.Path, takeover) -> None:
    proxy = start_session(session, "takeover")
    try:
        pid, window_id = find_window(session)
        time.sleep(5)
        tool(session, "get_window_state", pid=pid, window_id=window_id)
        tool(session, "start_recording", output_dir=str(session / "rec"), record_video=True)
        time.sleep(3)
        takeover(session)
        time.sleep(3)
        tool(session, "stop_recording")
    finally:
        call(str(session), {"stop": True})
        proxy.wait(timeout=20)


def scripted_takeover(session: pathlib.Path) -> None:
    mark(session, "takeover_start")
    set_page(session, "magenta")
    wait_for_colour(session, "magenta")
    time.sleep(3)
    mark(session, "takeover_end")
    set_page(session, "white")
    wait_for_colour(session, "white")


def host_json(*args: str) -> dict | list:
    return json.loads(subprocess.check_output(["hyprctl", "-j", *args]))


def host_eval(code: str) -> None:
    subprocess.run(["hyprctl", "eval", code], check=True, stdout=subprocess.DEVNULL)


def input_events(session: pathlib.Path) -> list[dict]:
    log = session / "page-events.jsonl"
    if not log.exists():
        return []
    events = [json.loads(line) for line in log.read_text().splitlines()]
    return [event for event in events if event.get("type") in ("pointerdown", "keydown")]


def person_takeover(session: pathlib.Path) -> None:
    """Verification step 4 and a real step 6, with a person at the desktop."""
    clients = host_json("clients")
    address = next(c["address"] for c in clients if c["class"] == "aquamarine")
    window_before = host_json("activewindow").get("address")
    workspace_before = host_json("activeworkspace")["name"]
    windows_before = sorted(c["address"] for c in clients)

    def go_to_workspace(name: str) -> None:
        host_eval(f'hl.dispatch(hl.dsp.focus({{ workspace = "{name}" }}))')

    def return_control() -> None:
        host_eval('hl.dispatch(hl.dsp.submap("reset"))')
        subprocess.run(["hyprctl", "setprop", f"address:{address}", "no_focus", "1"], check=True)
        go_to_workspace(workspace_before)
        if window_before:
            host_eval(f'hl.dispatch(hl.dsp.focus({{ window = "address:{window_before}" }}))')

    try:
        print(
            f"\n[1 of 3] Your desktop switches to the Session Window for {NO_FOCUS_CHECK_S} s. "
            "Click and type into it. Nothing should reach it."
        )
        input("Press Enter to start. ")
        go_to_workspace("name:keek")
        time.sleep(NO_FOCUS_CHECK_S)
        go_to_workspace(workspace_before)
        leaked = input_events(session)
        print(f"Events the page saw with no_focus set (want 0): {len(leaked)} {leaked}")

        print(
            "\n[2 of 3] Take control starts when you press Enter, and the page turns magenta.\n"
            "Type a few keys, click, then press a host shortcut such as Super+Return.\n"
            f"Super+Esc means Done. Control returns by itself after {TAKEOVER_LIMIT_S} s."
        )
        input("Press Enter to Take control. ")
        mark(session, "takeover_start")
        set_page(session, "magenta")
        wait_for_colour(session, "magenta")
        go_to_workspace("name:keek")
        subprocess.run(["hyprctl", "setprop", f"address:{address}", "no_focus", "0"], check=True)
        host_eval(f'hl.dispatch(hl.dsp.focus({{ window = "address:{address}" }}))')
        host_eval('hl.dispatch(hl.dsp.submap("keek"))')

        deadline = time.monotonic() + TAKEOVER_LIMIT_S
        while time.monotonic() < deadline:
            if subprocess.check_output(["hyprctl", "submap"], text=True).strip() == "default":
                break
            time.sleep(0.02)
        else:
            print("Time limit reached, so control was returned.")
        mark(session, "takeover_end")
        set_page(session, "white")
    finally:
        return_control()
    wait_for_colour(session, "white")

    received = input_events(session)[len(leaked):]
    windows_after = sorted(c["address"] for c in host_json("clients"))
    print(f"Events the page saw during the Takeover (want more than 0): {len(received)}")
    print(f"Host windows unchanged by the shortcut: {windows_before == windows_after}")
    print(f"Host submap: {subprocess.check_output(['hyprctl', 'submap'], text=True).strip()}")
    print(f"Focus returned: {host_json('activewindow').get('address') == window_before}")
    print(f"Workspace returned: {host_json('activeworkspace')['name'] == workspace_before}")
    input("\n[3 of 3] Check that Super+Return and other host shortcuts work again, then press Enter. ")


def magenta_frames(video: pathlib.Path) -> list[float]:
    """Returns the time in ms of each frame whose top decile of chroma is magenta."""
    output = subprocess.run(
        [
            "ffmpeg", "-hide_banner", "-i", str(video),
            "-vf", "signalstats,metadata=print:file=-", "-f", "null", "-",
        ],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    frames = []
    for block in re.split(r"(?=^frame:)", output, flags=re.MULTILINE):
        time_match = re.search(r"pts_time:([0-9.]+)", block)
        u_high = re.search(r"lavfi\.signalstats\.UHIGH=([0-9.]+)", block)
        v_high = re.search(r"lavfi\.signalstats\.VHIGH=([0-9.]+)", block)
        if not (time_match and u_high and v_high):
            continue
        if float(u_high.group(1)) >= MAGENTA_U and float(v_high.group(1)) >= MAGENTA_V:
            frames.append(float(time_match.group(1)) * 1000)
    return frames


def video_duration(video: pathlib.Path) -> float:
    output = subprocess.check_output(
        ["ffprobe", "-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", str(video)],
        text=True,
    )
    return float(output)


def takeover_times(session: pathlib.Path) -> tuple[float, float]:
    marks = {}
    for line in (session / "calls.jsonl").read_text().splitlines():
        entry = json.loads(line)
        if "mark" in entry:
            marks[entry["mark"]] = entry["t"]
    return marks["takeover_start"], marks["takeover_end"]


def cut(session: pathlib.Path, constant: float, guard: float) -> int:
    raw = session / "rec" / "recording.mp4"
    start, end = takeover_times(session)
    magenta = magenta_frames(raw)
    if not magenta:
        print(f"{session.name}: no magenta frame in the raw video, so this run proves nothing.")
        return 2
    cut_from = (start - constant - guard) / 1000
    cut_to = (end - constant + guard) / 1000
    output = session / f"cut-c{constant:g}-g{guard:g}.mp4"
    subprocess.run(
        [
            "ffmpeg", "-v", "error", "-y", "-i", str(raw),
            "-vf", f"select='not(between(t,{cut_from:.3f},{cut_to:.3f}))',setpts=N/FRAME_RATE/TB",
            "-an", str(output),
        ],
        check=True,
    )
    left = magenta_frames(output)
    raw_s = video_duration(raw)
    kept_s = video_duration(output)
    # A cut that removed far more than the Takeover would pass by erasing the video.
    removed_s = raw_s - kept_s
    expected_s = cut_to - cut_from
    if abs(removed_s - expected_s) > CUT_LENGTH_TOLERANCE_S:
        print(f"{session.name}: the cut removed {removed_s:.2f} s, not about {expected_s:.2f} s.")
        return 3
    print(
        f"{session.name}: takeover proxy {start:.1f}..{end:.1f} ms; "
        f"magenta in raw {magenta[0]:.1f}..{magenta[-1]:.1f} ms ({len(magenta)} frames); "
        f"cut {cut_from * 1000:.1f}..{cut_to * 1000:.1f} ms with C={constant:g} guard={guard:g}; "
        f"before start by {magenta[0] - cut_from * 1000:.1f} ms; "
        f"past end by {magenta[-1] - cut_to * 1000:.1f} ms; "
        f"removed {removed_s:.2f} s of {raw_s:.2f} s; magenta frames left: {len(left)}"
    )
    return 0 if not left else 1


def main() -> None:
    command, *args = sys.argv[1:]
    session = pathlib.Path(args[0]).resolve()
    if command == "run":
        record_around(session, scripted_takeover)
    elif command == "person":
        record_around(session, person_takeover)
    elif command == "cut":
        sys.exit(cut(session, float(args[1]), float(args[2])))
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
