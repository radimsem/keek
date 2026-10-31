# keek facts

What keek's design was checked against. The design itself is in [`design.md`](design.md).

## First spike results

Run on 2026-10-29 with a nested Hyprland under the developer's own user (no sandbox yet), Cua Driver 0.38 and the plugin loaded only into the nested instance. The agent signed in to a test page in Chromium:

- Click and type landed through the plugin's seat. Cua refuses background input to Chromium, so input needs that seat.
- Chromium exposed no accessibility elements, so the agent clicked by pixel.
- The recording shows Cua's cursor and click markers. Cutting by the Action Log took 20.3 s down to 8.9 s.
- The video ran a steady 1.45 s behind the Action Log.
- Leaks the sandbox now closes: the nested session inherited `DISPLAY=:0` and the developer's D-Bus and accessibility buses. Cua's `action.json` keeps typed passwords in plain text, which the credential scrub handles.

## Facts this design rests on

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
