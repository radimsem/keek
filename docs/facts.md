# keek facts

What keek's design was checked against. The design itself is in [`design.md`](design.md).

## First spike results

Run on 2026-10-29 with a nested Hyprland under the developer's own user (no sandbox yet), Cua Driver 0.38 and the plugin loaded only into the nested instance. The agent signed in to a test page in Chromium:

- Click and type landed through the plugin's seat. Cua refuses background input to Chromium, so input needs that seat.
- Chromium exposed no accessibility elements, so the agent clicked by pixel.
- The recording shows Cua's cursor and click markers. Cutting by the Action Log took 20.3 s down to 8.9 s.
- The video ran a steady 1.45 s behind the Action Log.
- Leaks the sandbox now closes: the nested session inherited `DISPLAY=:0` and the developer's D-Bus and accessibility buses. Cua's `action.json` keeps typed passwords in plain text, which the credential scrub handles.

## Sandbox spike results

Run on 2026-10-31 with Hyprland 0.56.2, aquamarine 0.15.1, `cua-hyprland-plugin` 0.33.2-3, Cua Driver 0.38.0, bubblewrap 0.13.1 and pasta 2026_10_02. The scripts and raw output are in [`spikes/sandbox/`](../spikes/sandbox/RESULTS.md).

The sandbox works: a Verifier subagent signed in to a test page in Chromium, recorded, with input through the plugin's seat. Inside there is no `DISPLAY` and the buses and namespaces are separate. Through the security-context connection, the screencopy and virtual input globals are hidden. Writes to the developer's config fail, and no sandbox process survives killing its parent.

Not run: `no_focus` against real clicks and keys, and a Takeover through the `keek` submap. Both need a person at the desktop, and #8 and #9 prove them.

Alignment, from 32 clicks across 4 sessions with the window hidden at 15 fps. A click's flip is its time in the video, and the page event is the timestamp the page reads when the click lands:

| Flip in the video, minus | Min (ms) | Median (ms) | Max (ms) | Spread (ms) |
| --- | --- | --- | --- | --- |
| The click call's start at the proxy | 18.9 | 59.3 | 333.0 | 314.1 |
| The click call's end at the proxy | -376.0 | -232.0 | -218.4 | 157.7 |
| The page event | -116.1 | -94.7 | -51.1 | 64.9 |

- A flip can't reach the video before the page acts, so the video's clock sits at least 116.1 ms behind the proxy's zero. keek's constant is 116 ms.
- A click call takes about 250 to 620 ms, and its effect lands at varying points inside it. That accounts for the 314 ms spread. Every flip fell inside its call, so the editor's windows, which keep the whole call plus margins, aren't affected.
- The 64.9 ms spread against the page event is about one frame at 15 fps, and it's what a Takeover cut has to absorb.

A scripted Takeover cut, scanning for magenta frames left from 45 or 46 in each of 3 runs:

| Constant (ms) | Guard (ms) | Magenta frames left |
| --- | --- | --- |
| 100 | 0 | 0, 1, 1 |
| 110 | 0 | 0, 0, 0 |
| 116 | 0 | 0, 0, 0 |
| 116 | 150 | 0, 0, 0 |
| 1536, the offset of Cua's turn clock | 0 | 21, 21, 21 |

At 116 and 150, the first magenta frame was at least 161.9 ms inside the cut and the last at least 185.5 ms inside it. The cut removed about 3.3 s for a 3.0 s Takeover.

What the sandbox needs beyond its plan:

- pasta closes inherited descriptors above stderr, so the process that opens the Wayland connection runs inside pasta: `pasta -- helper connect-exec -- bwrap`.
- pasta's defaults forward ports both ways and map the gateway to host loopback. A host listener on `127.0.0.1` was reachable from inside until `--no-map-gw -t none -u none -T none -U none`.
- `WAYLAND_SOCKET` must be set for Hyprland alone. Set for the whole sandbox, it reaches every bus-activated service. `dbus-run-session` still holds a copy of the descriptor, so a crashed Hyprland's window stays until the session ends.
- Bus-activated services inherit stdout, and a stray line corrupted the MCP stream, so that stream moves to private descriptors.
- `ATSPI_DBUS_IMPLEMENTATION=dbus-daemon` is needed, because the default `dbus-broker` fails without systemd.
- On NVIDIA, Hyprland aborts without `/dev/nvidia*` in the sandbox. `/etc` is mounted read-only, with a resolver file pasta can serve.
- The plugin opens its input socket only with `plugin:cua:enabled` set in the config at load. It refuses typing on any keymap but `evdev/pc105/us`.
- The plugin's `cua-input-v3-2.sock` lives under `$XDG_RUNTIME_DIR/hypr/<signature>/` and has to fit in `sun_path`. With a 25-character runtime folder it can't open, `hyprctl plugin list` still shows the plugin as loaded, and every `foreground` click fails with `production Hyprland input plugin is unavailable`. An 18-character folder fits, so keek always uses `/run/user/<uid>` inside the sandbox. Checked on 2026-11-01 with the versions above.
- Hyprland's Xwayland sets its own `DISPLAY`, never the host's. keek turns it off, which is what makes `DISPLAY` absent.
- `misc.disable_watchdog_warning` keeps Hyprland's warning out of the video.
- A hidden nested window needs a `render_unfocused` window rule to keep rendering, and then records at the host's `misc:render_unfocused_fps` (15 by default). A visible window wasn't measured.
- A Session Window's client pid on the host is that of the process that made the connection.
- Every input tool in 0.38.0 takes `delivery_mode`, with `background` as its default (`click`, `double_click`, `right_click`, `type_text`, `press_key`, `hotkey`, `scroll`, `drag`, `set_value`). `foreground` was run for `click` and `type_text`, and the driver requires `get_window_state` before pixel clicks.
- Driver variables, from `health_report` with each one removed: `CUA_DRIVER_RS_ENABLE_WAYLAND`, `WAYLAND_DISPLAY` and `XDG_RUNTIME_DIR` turn it degraded. Removing `HYPRLAND_INSTANCE_SIGNATURE` or `DBUS_SESSION_BUS_ADDRESS` still reports ok, but `health_report` doesn't test input.
- `xdg-desktop-portal-hyprland` used 95% of a core inside one early session. It wasn't reproduced or explained.

## Facts this design rests on

Checked on 2026-10-31 in Cua Driver 0.38.0 and `trycua/cua` main `a9902d2`:

- `cua-driver mcp` is a stdio MCP server that owns its runtime on Linux and shuts down on stdin EOF. Without a `session` argument, `start_recording` records every call on that connection.
- `CUA_DRIVER_RS_TELEMETRY_ENABLED=false` turns telemetry off. It is on by default.
- A tool's `read_only` flag is published as `annotations.readOnlyHint`. Cua records a turn for a tool that isn't read-only, except lifecycle and recording tools, `run_steps` and `run_script` (`tool.rs:2270`). Each step inside `run_steps` records its own turn.
- `recording_args_for` (`tool.rs:6367`) redacts `clipboard_write` text and `browser_dialog` prompts, but not `type_text`, `browser_type` or `set_value`.
- `end_session` runs the session's recording cleanup, so a Verifier calling it would end the recording.
- bubblewrap 0.13.1 has `--overlay-src`, `--overlay` and `--tmp-overlay`.

Checked on 2026-10-30:

- Cua Driver is a Rust MCP server and CLI under the MIT license, latest release 0.38.0 (pre-release). Its Hyprland support is listed as experimental. Its recorder takes one monotonic anchor for the video and cursor track, but reads turn times from a clock that starts after the video backend's 1.5 s startup probe ([#6210](https://github.com/trycua/cua/issues/6210)).
- `cua-hyprland-plugin` 0.33.2-3 is packaged by Omarchy and depends on exactly `hyprland=0.56.2-4` and `aquamarine=0.15.1-1`.
- A nested Hyprland's window always has class `aquamarine` and title `aquamarine - WAYLAND-1`; neither can be set (aquamarine v0.15.1 `src/backend/Wayland.cpp:558-559`).
- Its output follows every resize of that window and can't be pinned (`Wayland.cpp:541-556`, Hyprland v0.56.2 `src/output/Monitor.cpp:215-243`). A floating window of fixed size gives an output of exactly that size.
- Hyprland 0.56.2 implements `wp_security_context_v1`. Clients on such a socket see a whitelist of globals without screencopy, virtual keyboard or virtual pointer (`src/protocols/SecurityContext.cpp`, `src/managers/ProtocolManager.cpp:348-395`). A nested Hyprland binds only whitelisted globals.
- libwayland honours `WAYLAND_SOCKET`, and aquamarine connects with `wl_display_connect(nullptr)` (`Wayland.cpp:100`).
- `git worktree remove` deletes git-ignored files with the worktree, which is why Evidence lives outside the repository.
- bubblewrap 0.13.1 and pasta are installed on the reference machine.
- Cua Driver records native Wayland video with wf-recorder. Its docs list Hyprland as experimental, with input tested only for LibreOffice Calc and Inkscape; Chromium raw input is "not covered".
- Cua Cloud and Fleets ended on 18 October 2026. Cua Spaces runs containers, VMs and microVMs; its Linux app supports Ubuntu 24.04 only. Cua's Omarchy image is a stock Omarchy edge VM for amd64.
- `trycua/e2e-test-agent` (Apache-2.0) plans a PR journey, drives the app and writes a verdict, report and recording, in GitHub CI against a Cua desktop pool on X11.
- Cua's cursor is a plain arrow in the session's colour that floats while resting. Action marks, click ripple, glow, squish and rotation are settings that default to off.
- Claude Code passes an MCP server no caller identity. It aborts a local MCP call after 30 minutes with no response or progress (`CLAUDE_CODE_MCP_TOOL_IDLE_TIMEOUT`).

## Project Layer spike results

Checked on 2026-11-01 with bubblewrap 0.13.1, pasta 2026_10_02, Hyprland 0.56.2 and btrfs. The following checks ran on that host:

- bubblewrap's first `--overlay-src` is the bottom layer. Sessions use Project Layer, then worktree tree, then `--tmp-overlay`. Reversing the lower layers exposed stale install output over changed source. Files from the tree must win.
- Install uses `--overlay-src <tree> --overlay <upper> <work> <worktree>`. Upper-directory whiteouts and opaque directories survive renaming and work as lower layers with another tree. Overlay work directories can contain mode-000 `work/work`, so cleanup changes directory permissions first.
- pasta and bubblewrap pass the child's exit status through. pasta `-T` forwards namespace connections to the named host ports: the forwarded listener connected and another host listener was refused. `-T none` forwards none.
- The host Session Window belongs to connect-exec, pasta's child, rather than pasta itself. `/proc/<pasta pid>/task/<pasta pid>/children` lists that child on this kernel; other task children can be checked as well.
- Hyprland window resizing uses `hl.dispatch(hl.dsp.window.resize({ window = 'pid:N', x = W, y = H, relative = false }))`, after the client maps and before recording.

A real session on 2026-11-01 ran `python3 -m venv` and `pip install` in `install`: DNS and TLS worked, a first open took 5.5 to 7.0 s and a reuse 2.6 s, and a 640x480 setting produced a 640x480 Raw Recording. A venv's console scripts have an absolute shebang, which is why the worktree has a fixed path inside the sandbox.
