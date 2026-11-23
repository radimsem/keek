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

Run on 2026-10-31 with Hyprland 0.56.2, aquamarine 0.15.1, `cua-hyprland-plugin` 0.33.2-3, Cua Driver 0.38.0, bubblewrap 0.13.1 and pasta 2026_10_02. The scripts and raw output are in [`spikes/sandbox/` at `d594ee0`](https://github.com/radimsem/keek/blob/d594ee0ed5ed3d1198b33fd5f51b9167c4fd62e3/spikes/sandbox/RESULTS.md).

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

### Network reach results

Probed on 2026-11-03 in a throwaway pasta namespace, not a keek Agent Session, with pasta `2026_10_02.cba3570`, iproute2 7.3.0 and kernel 7.2.8. The namespace used keek's pasta flags, including `--config-net --dns-forward 169.254.1.1 --no-map-gw -t none -u none -T none -U none`.

- pasta's child was uid 0 in a new user namespace and `/usr/bin/ip` worked there. Host addresses and routes were copied, including a connected `192.168.1.0/24`, a gateway host route and a specific `10.200.0.1` route. A `/16` route would lose to the connected `/24` by longest-prefix matching; a `/16`-only block was not tested.
- Before the rules, the router at `192.168.1.1:80` and public `1.1.1.1:443` connected, and DNS at `169.254.1.1:53` returned 61 bytes. Host listeners on its own LAN IPv4 and IPv6 link-local addresses were refused; copied namespace addresses shadowed them.
- With the seven priority-20 unreachable rules and the priority-10 DNS exception, the router, `10.200.0.1:80` and `fe80::1` failed with `ENETUNREACH`. Public `1.1.1.1:443` connected and DNS still replied. Without the exception, DNS failed. A blackhole rule gave `EINVAL`; unreachable gave `ENETUNREACH`.
- Route probes blocked `172.31.255.255`, `100.64.0.1`, `169.254.9.9`, `fd00::1`, `fc12::1`, `fe80::5` and `febf::1`. They routed `172.32.0.1`, `100.128.0.1`, `169.254.1.1`, `fec0::1` and `2001:db8::1`. `127.0.0.1` and `::1` remained local.
- Inside another user namespace (`unshare -U -r`), deleting a rule failed with Operation not permitted and the five IPv4 blocks stayed.

Probed live on 2026-11-03 from inside an Agent Session by `live_project`, which passed, with passt 2026_10_02.cba3570-1:

- `ip route get` failed, and a TCP connect failed with `ENETUNREACH` in under 1 s, for `10.200.0.1`, `172.31.255.255`, `192.168.255.254`, `100.64.0.1`, `169.254.9.9`, `fd00::1` and `fe80::5`.
- `ip route get` succeeded for `172.32.0.1`, `100.128.0.1` and `1.1.1.1`.
- A DNS query to the `resolv.conf` nameserver got a reply.
- `ip -4 rule del to 10.0.0.0/8 pref 20` failed from inside the sandbox.

Not run: a listener on a non-template interface such as a VPN or Docker bridge. This host had no such address and creating one required root. These results do not establish behaviour on other pasta versions.

### Bare nested desktop results

Checked on 2026-11-09 with Hyprland 0.56.2, Omarchy 4.5.0-3, Quickshell 0.3.2 and Chromium 155.0.8059.84, by `live_session_shows_only_the_app`.

- Trial measurement, before the change: a 1440x960 Session Window with the bar and gaps held a window of 1388x882 at 26,52.
- With `bar-off` in the session's `toggles` and gaps, borders and rounding at 0, `list_windows` reported an ordinary Chromium window as 1440x960 at 0,0. Its page reported `innerWidth` x `innerHeight` of 1440x873, so Chromium's tab strip and toolbar take 87 px.
- `--kiosk --app=<url>` reported 1440x960 for the page.
- Omarchy's shell reads `bar-off` when it starts (`/usr/share/omarchy/shell/plugins/bar/Bar.qml`). A frame from the end of the Raw Recording showed no bar and no gap around the window.
- A critical `notify-send` sent 6 s into `start` showed as a card at the top right of that frame. One sent as `start`'s first command showed in no frame of a 6.7 s Raw Recording. The cause wasn't checked.
- The host's `toggles` folder listed the same entries before and after the session.
- With the developer's `decoration.active_opacity` of 0.97, the page's white background showed the wallpaper faintly through the window. With a window rule of `opacity = "1.0 override 1.0 override"` on every class, a 1000x700 crop of the page had one luma value.

### Chromium phone-width pages

Checked on 2026-11-09 with Chromium 155.0.8059.84 on the nested Hyprland:

- A plain `--kiosk <url>` window will not lay out below 500 logical px. A 390x844 Session Window crops a 500 px page. `--force-device-scale-factor` does not help because the minimum is in logical px.
- `--kiosk --app=<url>` opens an app window that takes the Session Window's exact size. The page reported `innerWidth` x `innerHeight` of 1280x800, 390x844 and 360x740 at those Session Window sizes, with no fullscreen exit bubble.

### Host window guard results

Checked on 2026-11-03 with Hyprland 0.56.2 and aquamarine 0.15.1, by `live_a_second_host_window_ends_the_session_as_incomplete`:

- `hyprctl output create wayland` from inside the sandbox opened a second `aquamarine` window on the host, owned by connect-exec's pid.
- The second window never took focus. The host's active window and workspace ended as they began.
- The gap from `openwindow` to `closewindow` for the second window was 32.9 ms, 59.3 ms and 46.7 ms in three runs.
- keek filed the session as an Incomplete Session for a host window, with no Verdict. No sandbox process was left.

### Caught window results

Checked on 2026-11-10 with Hyprland 0.56.2 and aquamarine 0.15.1, by `live_a_developers_nested_hyprland_returns_when_the_last_session_closes`. The test starts a nested Hyprland with a bare config from workspace 1 while one session is open:

- With the session open, the window was floating at 1280x800 on `keek` with `no_focus` true.
- Disabling the host rule alone turned `no_focus` false and left the window floating at 1280x800 on `keek`. The Hyprland wiki (read on 2026-11-10) lists `float`, `size` and `workspace` as static effects, evaluated once when the window opens, and `no_focus` as a dynamic one.
- With keek moving the window to the active workspace and tiling it after the last session closed, it ended tiled on workspace 1 at the size the layout gave it. The host's active window and workspace ended as they began.
- `ps -o comm=` names the Session Window's client process `keek` and the developer's `Hyprland`, which is how keek tells them apart.
- Not covered: whether the tiled window takes focus when the pointer rests where it lands. The host has `input:follow_mouse = 1`.

Seen once on the same day, with an earlier form of the test that sent SIGTERM: the nested Hyprland left its instance folder in `$XDG_RUNTIME_DIR/hypr`, holding `.socket2.sock` and `hyprland.log`. The test gives its nested Hyprland a runtime directory of its own.

### Takeover clipboard results

Read on 2026-11-03 at the aquamarine v0.15.1 and Hyprland v0.56.2 tags, with wl-clipboard 2.3.0 installed. The installed `0.15.1-1` and `0.56.2-4` builds were not compared with those tags.

- Source: aquamarine's Wayland backend binds only `wl_seat` v9, `xdg_wm_base`, `wl_compositor`, `wl_shm` and `zwp_linux_dmabuf_v1` (`src/backend/Wayland.cpp:119-137`), and takes only a keyboard and a pointer from the seat (`:348-366`). It never calls `get_data_device` and binds no primary selection or data-control manager, so a nested Hyprland has no path to the host selection, during a Takeover or outside one.
- Source: Hyprland 0.56.2 offers a security-context client `wl_data_device_manager` and primary selection (`src/managers/ProtocolManager.cpp:362`, `:387`), but not `zwlr_data_control_manager_v1` or `ext_data_control_manager_v1` (`:202`, `:218`).
- Source: the sandbox runs with `--clearenv`, a tmpfs over `XDG_RUNTIME_DIR` and its own network namespace, so the host's Wayland and D-Bus sockets are absent. `wl-paste` and `wl-copy` inside it see the nested clipboard only, and Cua Driver's `clipboard_read` and `clipboard_write` act on the nested clipboard too.
- Not covered: code loaded into the nested Hyprland as a plugin runs in the process that holds the host connection, and could bind `wl_data_device_manager` while the Session Window has focus. ADR 0006 places hostile code out of scope.
- Live: `live_a_takeover_keeps_the_host_clipboard_apart_from_the_nested_one` ran three times on 2026-11-03 and printed `Clipboard check not run: clipboard formats cannot be restored faithfully.` each time. The host clipboard held a copy from a Chromium-based app, which offers `chromium/x-source-url` and `chromium/x-internal-source-rfh-token` beside its text, and `wl-copy` can't put those back. The test left the clipboard, focus and workspace untouched.

Not run: the live Takeover clipboard check itself. It runs when the host clipboard holds only text or a single type.

### Live test cleanup results

Checked on 2026-11-03 with Hyprland 0.56.2, over every live test file:

- Each live test's daemon exited on SIGTERM within the 5 s grace: 12 daemons, none of which needed SIGKILL.
- After each file, no keek daemon was left and the host had no `keek-footer` layer, so no Session Footer stayed on the Agents Workspace.
- The host's focus, workspace and window count ended as they began.

### Notification results

Checked on 2026-11-09 in the Omarchy shell's notification plugin (`/usr/share/omarchy/shell/plugins/notifications`).

- A low urgency notification shows for 5 s and a normal one for 8 s, stretched to at most 30 s when the sender asks. A critical one stays until it is clicked or closed. In the first trial a normal "Agent needs you" notification expired after 7.4 s while the Help Request waited 3:12.
- `org.freedesktop.Notifications.CloseNotification` returns success but leaves a card on screen, because the shell keeps a visible card after its sender closes it. `omarchy-notification-dismiss <summary>` takes off every card whose summary contains that text.
- Do not disturb doesn't silence `omarchy-notification-send`, whose app name is `omarchy-action`.

### Opening Evidence results

Checked on 2026-11-08 with Hyprland 0.56.2, GTK 4.24.1, gtk4-layer-shell 1.3.0 and xdg-desktop-portal 1.22.1, from a gjs script outside keek. The daemon had exited with no log line and no core dump when Open video was pressed during the first trial.

- `gtk::FileLauncher::launch` with a gtk4-layer-shell window as parent gets `Error 71 (Protocol error) dispatching to Wayland display`, and GDK exits the process with status 1. A Session Footer is such a window, so the button ended the daemon and every open session.
- With no parent, or from an ordinary toplevel, the process survives, but the launch through the portal neither completed nor failed within 15 s, and no player opened.
- `gio::AppInfo::launch_default_for_uri` opened mpv, the default for `video/mp4`, at once.

### Still-screen results

Measured on 2026-11-08 with ffmpeg's `freezedetect` (`d=0.5`) on two Raw Recordings of the deployed login page at 1440x960, from the first trial.

- During a 4:43 Help Request wait, the code field's blinking caret, Cua's agent cursor and the login page's animated chart changed about 100 pixels every 1 to 3 seconds. At -60 dB that split the wait into 102 frozen pieces, so the editor showed it at real speed. At -55, -50, -45 and -40 dB it was one frozen span.
- At -55 dB the session with a wrong password still showed change while the Verifier typed and for 0.6 s as the error message appeared after the sign-in click.
- Only one machine and one app were measured.

### Takeover blur results

Checked on 2026-11-09 with ffmpeg's `gblur` on a 1440x960 Raw Recording from the first trial. The frame showed the authenticator step after the developer had typed all six digits of the code into its boxes.

- At `sigma=40` (height / 24, the editor's rule) no text on the page is readable, the six digits included. Only the page layout, the chart line and the button shapes remain.
- At `sigma=20` the six code boxes show only grey blobs.
- Only one app and one frame size were checked. Text larger than these 20 px digits needs a new check.

### Recording idle results

Checked on 2026-11-09 with Cua Driver 0.38.0 and `trycua/cua` at `b5adb8a35`, from the second trial, where a Verification Video ended 7.5 minutes before its session.

- Cua Driver stops its recording once 300 s pass without a tool call (`CUA_DRIVER_RS_RECORDING_IDLE_TTL_SECS`, `cua-driver-sdk/src/runtime.rs`). A thread checks every 30 s, so the cut lands 300 to 330 s after the last call. Any tool call resets it, `get_recording_state` included.
- The cut finalises a playable `recording.mp4`, and a later `stop_recording` still reports success, so nothing tells keek the recording ended early.
- A live session holding a Help Request with no Takeover recorded 329.8 s of 428.8 s. With Cua's TTL set to 5 s, a 60 s Help Request recorded 29.7 s of 68.6 s without a keepalive and 70.4 s of 69.0 s with a keepalive each second.

### Screen comparison results

Written on 2026-11-23 for the Takeover hand-back. The limit comes from the Still-screen results above. It was not measured on live `get_desktop_state` frames.

- keek takes one `get_desktop_state` screenshot once Take control completes, and another at Done. It decodes both with gdk-pixbuf (PNG or JPEG) and compares them pixel by pixel. gdk-pixbuf 2.44.8 on the development machine loaded PNG through glycin, and the fake-screenshot tests decode real PNGs through it.
- A pixel differs when any RGB channel differs by more than 16, which absorbs JPEG noise. Screens of different size count as changed.
- The screen looks unchanged when at most 0.02 % of its pixels differ: 276 pixels on a 1440x960 screen, 4 on the 200x100 test screens. The measured caret, agent cursor and animated chart changed about 100 pixels at a time.
- A baseline or a fresh screenshot that cannot be read counts as changed, so the Verifier gets today's note.
- The baseline is taken after Take control completes, so a developer who changes the screen before it is captured makes the comparison miss that change. A typed character that changes fewer than 276 pixels also reads as unchanged. Both cost the Verifier one re-read of the screen.

Not run: a live Takeover with a real Help Request, so the limit is untested against real frames.

### Third trial results

Checked on 2026-11-09 with Cua Driver 0.38.0, Hyprland 0.56.2 and Chromium 155.0.8059.84, from the third trial.

- `drag` with `delivery_mode` set to `foreground` drew a precise rectangle on a pdf.js canvas on the first try, at 305 % zoom on an unrotated page and at 244 % on a view rotated 90 degrees.
- Chromium's GTK file dialog (xdg-desktop-portal-gtk) accepted Ctrl+L, a typed absolute sandbox path and Enter. A .pdf filter did not block a typed .txt path.
- `${NAME}` substitution ran in four sessions, and a grep of all Evidence and logs for the value found nothing.
- `type_text` into a password field returns `effect: unverifiable`, and the field can't be read back.
- `get_window_state` on the portal's file dialog failed once with `surface_identity_unproven` and worked in another session. Seen once, not reproduced.

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

Checked on 2026-11-03 against the Hyprland wiki:

- A Hyprland 0.56.2 window rule matches class, title, tag, workspace and window state, never a pid. A dispatcher selector takes `pid:...`.

Checked on 2026-11-03 with bubblewrap 0.13.1 and pasta 2026_10_02.cba3570:

- bwrap's `--args FD` accepts `--setenv`. It reads NUL-separated arguments from `FD` and parses them where the option stands, so an earlier `--clearenv` still wins and a later `--setenv` still overrides. Values keep spaces, tabs, quotes and `=`, and an empty file adds no arguments. bwrap reads from the descriptor's current offset and reports no error when that offset is already at the end, so a writer must rewind first. bwrap closes `FD` after reading, refuses a nested `--args`, and counts the expanded arguments toward its 9000 limit (`parse_args_recurse`, `bubblewrap.c`).
- pasta passes its own environment to the command it starts, so a value can reach that command without appearing in any argument list.
- bwrap's `--disable-userns` can't be used with a Chromium App Under Test. It sets `user.max_user_namespaces=1` and enters a nested user namespace, so a nested `bwrap` or `unshare -U` inside fails with ENOSPC, which also takes away Chromium's own sandbox; its SUID fallback then aborts, because `chrome-sandbox` is not root-owned in a sandbox mapping only one uid. `list_windows` never finds the window. Chromium runs if it is started with `--no-sandbox`, which keek can't ask of a Project's Setup Script. Checked on 2026-11-03 with bubblewrap 0.13.1 and Chromium from Omarchy.

## Project Layer spike results

Checked on 2026-11-01 with bubblewrap 0.13.1, pasta 2026_10_02, Hyprland 0.56.2 and btrfs. The following checks ran on that host:

- bubblewrap's first `--overlay-src` is the bottom layer. Sessions use Project Layer, then worktree tree, then `--tmp-overlay`. Reversing the lower layers exposed stale install output over changed source. Files from the tree must win.
- Install uses `--overlay-src <tree> --overlay <upper> <work> <worktree>`. Upper-directory whiteouts and opaque directories survive renaming and work as lower layers with another tree. Overlay work directories can contain mode-000 `work/work`, so cleanup changes directory permissions first.
- pasta and bubblewrap pass the child's exit status through. pasta `-T` forwards namespace connections to the named host ports: the forwarded listener connected and another host listener was refused. `-T none` forwards none.
- The host Session Window belongs to connect-exec, pasta's child, rather than pasta itself. `/proc/<pasta pid>/task/<pasta pid>/children` lists that child on this kernel; other task children can be checked as well.
- Hyprland window resizing uses `hl.dispatch(hl.dsp.window.resize({ window = 'pid:N', x = W, y = H, relative = false }))`, after the client maps and before recording.

A real session on 2026-11-01 ran `python3 -m venv` and `pip install` in `install`: DNS and TLS worked, a first open took 5.5 to 7.0 s and a reuse 2.6 s, and a 640x480 setting produced a 640x480 Raw Recording. A venv's console scripts have an absolute shebang, which is why the worktree has a fixed path inside the sandbox.

## Toolchain Store results

Checked on 2026-11-04 with `/usr/bin/mise` 2026.10.25 linux-x64 (2026-10-31), against mise's documentation for v2026.10.21, in a scratch folder with `env -i`. Only jq was installed; node, rust, python, pnpm and yarn were not installed by this check.

- `MISE_DATA_DIR`, `MISE_CONFIG_DIR`, `MISE_CACHE_DIR`, `MISE_STATE_DIR`, `MISE_SYSTEM_CONFIG_DIR` and `MISE_GLOBAL_CONFIG_FILE` move each of mise's folders. mise also writes under `$HOME/.local/share/mise`, so keek sets `HOME` inside the store too.
- With only the folders above moved, mise still read `/etc/mise/conf.d/omarchy.toml`, and `mise install` failed on a tool that file names. Pointing `MISE_SYSTEM_CONFIG_DIR` at an empty folder fixed it.
- Before any install, `mise where node@24` resolves the pin against the newest remote release (24.22.2) and fails with "not installed". After an install, `mise where jq@1` reports the newest installed match, `.../installs/jq/1.7`, whose folder name is the version.
- `mise install jq@1.7` when it is present prints "already installed" and downloads nothing. `mise bin-paths jq@1.7` works for an installed tool that no config activates.
- mise's rust backend uses `RUSTUP_HOME` and `CARGO_HOME`; the settings `rust.rustup_home` and `rust.cargo_home`, or `MISE_RUSTUP_HOME` and `MISE_CARGO_HOME`, take precedence, and with explicit homes mise runs its own rustup-init. Toolchains live in `RUSTUP_HOME`. A rust pin inside the sandbox has not run.
- The registry maps node to `core:node`, python to `core:python`, rust to `core:rust`, pnpm to `aqua:pnpm/pnpm` and yarn to `vfox:jdx/vfox-yarn`.
- On this host `/usr/bin/node` is v26.12.0, and there is no `/usr/bin/cargo`, `pnpm` or `yarn`. The developer's `~/.config/mise/config.toml` and `~/.local/share/mise/installs` exist and were left unchanged.

## Cua Driver refusal texts

Checked on 2026-11-09 with Cua Driver 0.38.0, from the third trial. Each arrives as text content with `isError` true.

- `click` without a pid: `Missing required integer field: pid`
- `get_window_state` for a pid that owns no window: `No windows found for pid 158. Provide window_id.`
- `click` with an unknown capture: `capture-bound click refused: capture id is unknown`

The three texts were seen in the third trial. `live_click_with_only_window_id_lists_the_windows` measured the first on 2026-11-09, and that `list_windows` called with `{}` returns its list as one text item in this shape:

```
Found 1 windows:
  window_id=94036789428144 pid=158 "keek fixture - Chromium" 1691x1298+0+0 app=chromium
```

That `list_windows` is read-only, so Cua records no turn for it, is read from source. A Cua Driver version change rechecks these texts.

## Omarchy startup cover

Checked on 2026-11-14 against Omarchy 4.5.0-3 source:

- `/usr/share/omarchy/shell/services/BackgroundIntro.qml` maps a full-screen black startup cover with namespace `omarchy-background` at the overlay level (3).
- It skips the startup cover when the trimmed text of `~/.local/state/omarchy/background-intro.session-id` equals `OMARCHY_SESSION_ID` (falling back to `HYPRLAND_INSTANCE_SIGNATURE` when the variable is empty). `omarchy-theme-bg-boot-intro` also exits early for a matching marker.
- A 10 second fallback timer releases the startup cover when the background or shell does not become ready. An install eval saw the cover hide the app for 6 to 14 seconds.

Checked the same day in a nested Agent Session, by `live_the_first_recorded_frame_shows_the_app_instead_of_a_black_cover` on a white fixture page:

- With a marker that didn't match and no wait, the first frame of the Raw Recording had a mean luma of 14, which is video black.
- With a matching marker bound read-only and `OMARCHY_SESSION_ID` set, the test passed and `open_session` took no longer.
- With a marker that didn't match, waiting until no overlay-level `omarchy-background` layer was left also passed, and the test took about 5 s longer.
