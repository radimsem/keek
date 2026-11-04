# keek design

keek lets a coding agent have its change verified by a fresh Verifier that uses the App Under Test like a person would, on a sandboxed nested Hyprland desktop inside the developer's own Omarchy session, and leaves Evidence a reviewer can attach to a pull request. Terms are defined in [`GLOSSARY.md`](GLOSSARY.md); decisions that are hard to reverse are in [`docs/adr/`](adr/).

The model is Warp's factories pipeline: the agent's verification is recorded, and the stretches where it is only reasoning are squeezed out of what the reviewer watches.

## Scope

v1:

- Your own projects (web apps in a browser, native Wayland GUI apps), run on the developer's own machine. An X11-only app can't run, because Xwayland is off in the sandbox.
- Omarchy is the only supported and tested host. An Agent Session runs the host's own Hyprland, apps and Omarchy config, nested inside the developer's session.
- One Agent Session per git worktree, at most two open at once (configurable). Each open session takes a slot on the Agents Workspace.
- Evidence stays local, under `~/Videos/keek`. The agent prints its paths; the developer attaches the Verification Video to the PR by hand.

Later:

- Testing Omarchy itself: a Setup Script that mounts an Omarchy worktree's `bin/`, `config/` and `shell/` into the Agent Session in place of the developer's own. The design keeps this seam open and does nothing else for it in v1.
- Upload delivery (storage you configure, linked from the PR body).

Out of scope:

- Omarchy core integration until keek is stable ([ADR 0002](adr/0002-standalone-not-omarchy-core.md)).
- CI runners. keek verifies on the developer's machine, where the live view and Takeover matter; Cua's `e2e-test-agent` covers CI.

## Built on Cua

keek is an Omarchy layer on Cua Driver ([ADR 0009](adr/0009-shared-pieces-go-to-cua-driver.md)). Cua Driver performs Actions and records them. keek adds the sandboxed nested session on the developer's own Omarchy, the viewer and Takeover, its MCP tools and Evidence filed by Project.

keek runs stock Cua Driver 0.38.0 and doesn't wait for upstream changes ([ADR 0013](adr/0013-keek-times-the-session-on-stock-cua-driver.md)). Three pieces still belong in Cua Driver. Until a Cua release carries them:

| Piece | Today in Cua Driver 0.38.0 | Upstream | keek meanwhile |
| --- | --- | --- | --- |
| Hyprland video aligned with the Action Log | Turn times read about 1.45 s behind the video. | [#6210](https://github.com/trycua/cua/issues/6210), [PR #6239](https://github.com/trycua/cua/pull/6239) | The Session Timeline times Actions itself. This stays after the fix. |
| Idle Fast-forward in `cua-driver recording render` | `render` zooms around each Action; it has no Fast-forward. | [#6211](https://github.com/trycua/cua/issues/6211) | Its own ffmpeg editor (`stopgap`), removed once a release carries #6211 and the #6239 fix. |
| Masking typed secrets in `action.json` | Typed text is stored in plain text. | [#6250](https://github.com/trycua/cua/issues/6250) | Scrubs Test Credentials when filing Evidence (`stopgap`), removed once a release carries masking. |

Issue and pull request text is approved by the developer before it is posted.

## Isolation

An Agent Session is a nested Hyprland in a bubblewrap sandbox under the developer's uid ([ADR 0006](adr/0006-nested-hyprland-in-a-sandbox-under-your-uid.md)). The coding agent already runs as the developer with a shell, so the sandbox keeps the agent's desktop from disturbing the developer's. It is not a wall against a hostile agent.

| Resource | In an Agent Session |
| --- | --- |
| Screen, input, focus | Its own nested Hyprland. The agent's input arrives on the Cua plugin's seat. The host shows the session as a Session Window that ignores the developer until Take control. |
| Host compositor | One connection, passed in as `WAYLAND_SOCKET` and created through a `wp_security_context_v1` socket, so it can't use screencopy or virtual input against the host. The daemon ends the session if this connection opens another window or its Session Window changes class. It checks on `openwindow` and every second; the window can exist until detection and sandbox termination. |
| Desktop services | Its own D-Bus session bus and accessibility bus, and its own Omarchy shell, notifications and clipboard inside the nested desktop. The nested desktop binds no host clipboard or primary selection, so neither crosses in either direction, including during a Takeover. This rests on source reading; the live check has not run ([`facts.md`](facts.md#takeover-clipboard-results)). No `DISPLAY`, the host's or its own: Xwayland is off. |
| Home and config | A throwaway HOME. The developer's Hyprland and Omarchy config are mounted read-only, with keek's overrides on top: scale 1, the Cua plugin and the `us` keymap it requires, Xwayland off, no idle lock. |
| Session folder | `~/.cache/keek/sessions/<session>/`, bind-mounted writable, so Cua Driver records where the host can render and file. |
| Worktree | A throwaway write layer over a tree of tracked and untracked files, over the Project Layer, in that order from top to bottom. The sandbox mounts it at `/keek/worktree`, so a Project Layer works from any worktree. `.git` is absent, so `git` does not work inside. The tree wins over install writes to tracked paths so a Verifier sees the agent's change. Git-ignored files (`.env*`, local builds, `node_modules`) are hidden unless the Setup Script creates them during install. |
| Network | A private network namespace through pasta: outbound internet and its own localhost. Before an Agent Session's sandbox starts, keek makes `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`, `100.64.0.0/10`, `169.254.0.0/16`, `fc00::/7` and `fe80::/10` unreachable using policy rules that sandboxed code cannot remove. The DNS forwarder at `169.254.1.1` is excepted, on every port. If `ip` is missing or any rule fails, including on a host with IPv6 turned off, keek refuses the Agent Session and names the failing command in `sandbox.err`. A Project reaches a host loopback service through a forwarded port; a LAN service can use a host loopback tunnel and a forwarded port. LAN neighbours with public addresses and special ranges outside this list remain reachable. The install phase keeps LAN access so a Setup Script can reach a private package mirror. |
| Shared with the host | Kernel, uid, installed packages and toolchains, GPU. |

## Components

keek is one Rust binary ([ADR 0014](adr/0014-keek-is-one-rust-binary.md)).

- **Nested Hyprland**: the host's installed Hyprland, started inside the sandbox. Its output always takes the size of its window on the host, so keek floats that window at a fixed size ([ADR 0007](adr/0007-nested-window-is-the-session-window.md)).
- **cua-hyprland-plugin**: from Omarchy's package repository, which builds it against exact Hyprland and aquamarine versions. `open_session` checks that the plugin loaded into the nested Hyprland and refuses otherwise, naming both versions.
- **Setup Script**: committed to the repository as `.keek/setup.sh`, one per repository. keek runs it inside the sandbox with `KEEK_PROJECT` set to the Project name, in two phases *(my call: the two phases)*: `install` (cached as the Project Layer) and `start` (starts the App Under Test). A single-app repository ignores `KEEK_PROJECT`. `open_session` refuses when `.keek/setup.sh` is missing, git-ignored or not a regular file, and asks the agent to commit a Setup Script with both phases. A non-zero `start` does not refuse an Agent Session; its output goes to `start.log`.
- **Project Layer**: the `install` phase's writes, cached under `~/.cache/keek/projects/<project>/` and keyed by SHA-256 of length-framed Setup Script content and sorted tracked lockfile paths and contents read from disk. Uncommitted edits change the key; untracked lockfiles do not. An exclusive lock for the Project serializes check and build. A build starts from the copy of the worktree files, writes into `.build/upper`, then renames that upper directory to the key on success. Writes outside the worktree, including HOME and `/tmp`, are discarded. HOME is replaced, so toolchains under the real HOME, such as mise, rustup and nvm, are not visible. Failed or timed-out installs remove `.build` and refuse `open_session` with the last 4 KiB of the host-owned `install.log`; install has a separate 20 minute limit before the desktop handshake.
- **Worktree files**: `open_session` refuses a repository whose git top level is not the folder holding its `.git`, which is what a `core.worktree` setting does, so a repository's config can't choose the host folder that is copied. `git ls-files -co --exclude-standard -z` supplies raw paths, sorted and deduplicated. Regular files use reflink, then copy if reflink is unavailable, symlinks retain their target, and nested repositories and submodules become empty directories. Missing files and special files are skipped. The tree lives at `~/.cache/keek/trees/<session>/`, outside the writable session folder so the sandbox cannot modify an overlay lower layer. Closing or refusing an Agent Session removes its tree. A daemon crash can leave one behind.
- **Per-Project settings**: `~/.config/keek/projects/<project>.toml`, set up by hand on each machine: the Session Window size and the host ports to forward into both install and session sandboxes. Missing settings use the 1280x800 window and no forwarded ports. Unknown keys, zero ports and zero window dimensions refuse `open_session` with the file path and a request to fix it. After the driver handshake and plugin check, keek resizes the Session Window by the connect-exec pid before recording starts.
- **Test Credentials**: `~/.config/keek/projects/<project>.env`, injected into the sandbox as environment variables for the App Under Test. Their values reach the sandbox through connect-exec's environment and one arguments file descriptor, so no process's command line carries them. The Verifier gets only their names from `open_session`. It types `${KEEK_TEST_PASSWORD}`, and keek substitutes the value before forwarding the call and masks it in what comes back. Cua Driver still records the value, so keek scrubs every credential value from the Action Log's text files when it files Evidence, and removes the session's cache folder once filed. Screenshots can't be scrubbed, which is why Test Credentials are test-only. The env file has one `NAME=value` per line, with blank lines and comments skipped and an optional `export ` prefix. Names use `[A-Za-z_][A-Za-z0-9_]*`. Matching single or double outer quotes are removed literally without escape processing; unquoted values are trimmed. Duplicate names and values shorter than 6 characters refuse opening. A mode readable by group or others warns to run `chmod 600 <path>`. Masking replaces raw, JSON-escaped (once or twice), and percent-encoded forms with `[masked]`. Filing scrubs UTF-8 files recursively inside direct `turn-*` folders and `timeline.json`, leaving binary files unchanged. A scrub failure, including a file over 64 MiB, withholds the Action Log, returns `actions_error`, and retains the session folder for the developer to check and delete.
- **Host MCP server**: the only thing keek ships to the coding agent (`claude mcp add keek -- keek mcp`, or the equivalent in another harness; [ADR 0011](adr/0011-mcp-server-only.md)). Its instructions describe the Verifier protocol. It is a thin client of the keek daemon and starts one if none is running.
- **keek daemon**: one per user ([ADR 0015](adr/0015-one-per-user-daemon-owns-sessions.md)). It owns every open Agent Session, the session cap, the slots, the Session Footers and the connection to each Cua Driver, and it keeps the Session Timeline. It exits once no client, session or Session Card remains.
- **Cua Driver** (inside the sandbox): performs Actions through the plugin's input seat and records the session ([ADR 0003](adr/0003-cua-driver-owns-input-and-recording.md)). It runs inside so that it sees the sandbox's accessibility bus and nested display. Its config lives in the sandbox's throwaway HOME, so keek's cursor settings never touch the developer's own. keek mounts the pinned binary from an explicit path, because `~/.local/bin` sits under the HOME the sandbox replaces, and turns the driver's telemetry off.
- **Renderer** (on the host): keek's ffmpeg editor, cutting from the Session Timeline, plus keek's Takeover title cards. Cua Driver's `recording render` replaces the editor once a release carries Fast-forward.

Settings use this format; `ports` and `window` are optional, and a supplied window has two non-zero dimensions:

```toml
ports = [5432, 6379]

[window]
width = 390
height = 844
```

`open_session` always returns a `warnings` array. Each missing settings or Test Credentials file adds a warning naming the Project and path.

## Viewer

The developer watches Agent Sessions and takes them over on the Agents Workspace, where each Session Window is the nested Hyprland's own window and its Session Footer is joined beneath it; see [`viewer.md`](viewer.md).

## Verifier protocol

The coding agent never drives an Agent Session itself ([ADR 0010](adr/0010-a-fresh-verifier-checks-each-change.md)). keek's server instructions tell it to:

1. Write the goal as a short list of acceptance checks a person could verify on screen ("Signing in with the test account lands on the dashboard"). They become the Verification Checklist.
2. Start a fresh subagent, the Verifier, with only the goal and its checks, the Project name and the worktree. No diff and no notes on what changed.
3. Read the Verifier's Verdict and Evidence paths. A Verdict is final.
4. After Needs fixes, read the failed Checks and their comments, fix the change and start a new Verifier, up to 3 Agent Sessions per change. Then stop and report to the developer with every session's Evidence. After Partly verified, tell the developer which Checks were skipped and why.

`open_session` returns `credentials`, the names of the Project's Test Credentials. To use one, write `${NAME}` in a tool argument, including inside `run_steps`. keek substitutes the value before forwarding and masks it in results, so the Verifier never sees it. An unknown name is refused. In `run_script`, substitution is plain text inside JavaScript, so prefer `run_steps` for a value with quotes or backslashes.

keek can't tell which agent calls its tools, so these steps hold by instruction only.

## Agent interface

The host MCP server exposes these to the Verifier. MCP calls carry no caller identity and one coding agent can run two Verifiers, so every tool but `open_session` takes the session's name as a required `session` argument:

- `open_session(project, worktree, goal, checks)`: refuses if this worktree already has an open Agent Session or the cap is reached. Any Project name is accepted, and one repository can hold several Projects ([ADR 0008](adr/0008-agent-names-the-project.md)). It records the worktree's branch, commit and whether it has uncommitted changes. It then:
  1. builds or reuses the Project Layer
  2. starts the sandbox and nested Hyprland in a free slot
  3. injects Test Credentials and starts the App Under Test
  4. starts recording and the Session Timeline

  It returns once the desktop is ready, with the session's name (`myapp_14-03-22`), each Check's id and text, and the names of its Test Credentials. It warns when the Project has no Test Credentials or settings, which is how a misspelled name shows up.
- Cua Driver's tools (`get_window_state`, `click`, `type_text` and the rest), forwarded with `session` added to their schema and stripped before the call. These are not forwarded, because each would undo something keek owns:
  - Recording is always on: `start_recording`, `stop_recording`, `get_recording_state`, `replay_trajectory`.
  - keek owns the driver session, whose end stops the recording: `start_session`, `end_session`, `escalate_session`.
  - keek owns the driver's setup: `set_config`, `check_for_update`, `install_extension`, `install_ffmpeg`, `set_agent_cursor_enabled`, `set_agent_cursor_motion`, `set_agent_cursor_theme`.

  A forwarded tool with a `delivery_mode` argument defaults to `foreground`, in its schema and when keek forwards the call. Cua's default is `background`, which Chromium refuses, and foreground input stays inside the nested desktop, so it can't disturb the developer.
- `request_help(session, reason)`: raises a Help Request and blocks until the developer finishes a Takeover; see [`viewer.md`](viewer.md#takeover). While it waits it sends progress notifications, because Claude Code aborts a local MCP call after 30 minutes with no response or progress.
- `add_check(session, text)`: adds a Check the Verifier found worth checking while testing. It is marked as added in the Verdict Report and on the card.
- `mark_check(session, check, outcome, comment)`: marks a Check passed, failed or skipped; failed and skipped need a one-line comment. A Check can be marked again until the session closes. keek pushes each mark to the Session Footer and logs it in the Session Timeline with its time.
- `close_session(session, summary)`: refuses while a Check is unmarked and names the unmarked ones, and refuses a session with no Checks, which would otherwise Pass with nothing verified. It is decided ahead of queued calls, after any `add_check` and `mark_check` already queued. Once it passes, it stops a call in flight and refuses the queued calls; a refused close leaves them running. A held Help Request, a Takeover and its hand-back capture finish first. keek derives the Verdict from the Checks ([ADR 0012](adr/0012-verdict-follows-from-the-checklist.md)), stops recording, stops the sandbox, renders, files the Evidence and returns the Verdict, every Check with its outcome and comment, and the Evidence paths. If keek cannot stop the recording, it files an Incomplete Session in place of the Verdict, and the reply carries `incomplete`. If the session opens a window on the developer's desktop besides its Session Window, it is also incomplete, even if recording stops successfully. A sandbox close error after a good stop keeps the Verdict unless the session opened such a window. If the editor fails, keek still files the rest of the Evidence and returns `video_error`, and the Verdict Report says its Check times are positions in the Raw Recording.

## Recording and editing

Recording starts in `open_session` and stops in `close_session`; the Verifier has no recording tool. Cua Driver's `start_recording` with `record_video:true` writes, per session:

- `recording.mp4`: the Raw Recording (wf-recorder on the nested output, CPU encoded). While the Session Window is hidden on the Agents Workspace, it records at the host's `misc:render_unfocused_fps`, 15 by default. keek leaves that setting alone.
- `turn-NNNNN/action.json` per Action: tool, arguments, result, click point, and `t_ms_from_session_start`.

Read-only calls (snapshots, accessibility trees) are not Actions and leave no turn, so the gaps between turns are where the Verifier was reasoning.

keek sets Cua's cursor for every session: action marks on (clicking, typing, scrolling), a ripple on each click, and the resting float off. Cua's default cursor floats while it rests, which would keep an idle screen from ever being still. keek draws no overlay of its own.

### Session Timeline

The first spike found the video running a steady 1.45 s behind Cua's turn times. keek doesn't use those times for anything on the video. It keeps its own Session Timeline at the MCP proxy ([ADR 0013](adr/0013-keek-times-the-session-on-stock-cua-driver.md)):

- keek reads a monotonic clock just before it forwards `start_recording`, and counts every later time from there, less a constant of 116 ms, because the video's clock sits that far behind this zero ([`facts.md`](facts.md#sandbox-spike-results)).
- Each forwarded call gets its start and end. A call is an Action when its tool isn't marked `readOnlyHint`, the same rule Cua uses for turns. One `run_steps` call is one active span, even though Cua logs a turn per step. A click's effect lands somewhere inside its call, up to 333 ms after its start, so nothing treats a call's start as the moment its effect shows.
- Check marks and Takeovers go on the same timeline.

It is data only in v1: the editor and the Verdict Report read it, and nothing draws it. Cua's `t_ms_from_session_start` stays in the Action Log as recorded, so on 0.38.0 it reads about 1.45 s behind the video.

### Verification Video

The Verification Video ([ADR 0004](adr/0004-fast-forward-idle-instead-of-cutting.md)) comes from keek's ffmpeg editor until Cua Driver's `recording render` has the same edit:

1. Keep a window from 0.3 s before each Action's start to 1.5 s after its end.
2. Extend a window while the screen keeps changing (frame difference or ffmpeg `freezedetect`), so animations, loads and late toasts stay at real speed.
3. Merge overlapping windows. Everything left is an Idle Stretch.
4. Show each Idle Stretch as a Fast-forward of about 0.6 s, labelled with the real time it took (`+18 s`).

keek splits the session at each Takeover, edits each part, and joins the parts with a title card in place of each Takeover ("You were in control · 0:42 · 2FA code"). Each cut reaches 150 ms past the Takeover on both sides, about twice the 65 ms the video varies against a page's own timestamps. A leaked frame could show a 2FA code, while a longer cut costs a moment beside a title card. The same split stays when Cua's render replaces the editor, because that render knows nothing of Takeovers. The Raw Recording keeps the footage.

The margins are starting values to tune against real sessions. The spike's five Actions took 20.3 s raw and 8.9 s edited.

## Evidence

While a session is open, Cua writes into `~/.cache/keek/sessions/<session>/`. `close_session` files the Evidence by Project and date in the developer's videos folder (`xdg-user-dir VIDEOS`, normally `~/Videos`):

```
~/Videos/keek/
└── myapp/
    ├── index.md
    └── 2026-10-30/
        ├── myapp_14-03-22_needs-fixes_actions/
        ├── myapp_14-03-22_needs-fixes_raw.mp4
        ├── myapp_14-03-22_needs-fixes_report.md
        ├── myapp_14-03-22_needs-fixes_timeline.json
        ├── myapp_14-03-22_needs-fixes_verification.mp4
        ├── myapp_16-41-07_pass_actions/
        └── …
```

- Names are `<project>_<HH-MM-SS>_<verdict>_<type>.<ext>`. The date folder and time use the local date and time the session started. The verdict is `pass`, `needs-fixes`, `partly-verified` or `incomplete`. A second session of the same Project starting in the same second gets `-2` after the time.
- `verification.mp4` is the Verification Video, filed once the editor has produced one, `raw.mp4` the Raw Recording, `timeline.json` the Session Timeline, and `actions/` the Action Log with Test Credentials scrubbed.
- `report.md` is the Verdict Report. Its header names the goal and the revision ("Verified fix/17-parser-bom at a1b2c3d + uncommitted changes"). It lists the Verification Checklist, each Check with its outcome, comment and timestamp in the Verification Video, then the Verifier's summary and the Verdict. It states that its times come from the Session Timeline, not from `actions/`. It is ready to paste into a PR body after review.
- `index.md` gets one line per session, appended at close: date, time, branch, commit, Verdict, goal, and links to the files.
- keek never deletes Evidence.

Every file carries the Project and time in its name, so a Verification Video dragged into a PR still says where it came from.

## Abnormal end

An Agent Session can end without `close_session`: the Verifier finishes, crashes or runs out of context first, the coding agent's session exits, or the developer presses Ctrl-C. keek's MCP server belongs to the coding agent's session, so a Verifier ending ends no process keek can watch. Every Cua call passes through keek, so the daemon ends the session after 5 minutes without a tool call, outside a Help Request or Takeover, and when the `keek mcp` that opened it disconnects. It stops recording, stops the sandbox and renders anyway. If the daemon itself dies, the sandbox dies with it, and the next daemon to start finishes any session it finds unfiled the same way. The window rule keek installs for Session Windows matches every `aquamarine` window, including a developer's own nested Hyprland, so keek disables it when the last session ends, when the daemon stops on SIGTERM, and when the next daemon starts. The daemon also ends a session that opens a window on the developer's desktop besides its Session Window, including a second `aquamarine` window, or changes its Session Window's class. It compares host client pids with connect-exec's pid, checks on `openwindow` and polls every second for class changes. Detection and sandbox termination leave a short gap during which the window exists. The session is an Incomplete Session. Its files carry `incomplete` in place of a verdict, its Verdict Report says why it ended, lists the Checks marked so far and has no Verdict, and its Verification Video is labelled incomplete. While its Session Card remains, a call that names the session is refused with the reason and the Evidence folder, and the Verifier is told to open a new Agent Session.

An Agent Session can also end in a `close_session` that cannot stop the recording. It is an Incomplete Session because recording could not be stopped, unless it opens a window on the developer's desktop besides its Session Window, which takes precedence as the reason. A sandbox close error after a good stop keeps the Verdict unless the session opened such a window.

keek keeps an atomic, host-only journal for each open session under `$XDG_STATE_HOME/keek/sessions/`, because the sandbox can change files in the session folder and a retained folder may already be filed. After a daemon stops, the next daemon files the journaled session as incomplete and scrubs its Action Log only if the Project's Test Credentials file still has the same bytes, with both files absent counting as unchanged. If the file changed or was removed, keek withholds the Action Log and explains why in the Verdict Report.

If a Takeover was still open when the daemon stopped, recovery cuts from the start of that Takeover to the end of the recording.

The spikes' results and the facts this design rests on are in [`facts.md`](facts.md).

## Risks

- Cua's Hyprland support is experimental, and with no accessibility tree in Chromium the agent clicks by pixel. Scale 1 in the nested desktop keeps coordinates right (Cua issues #4944, #3061).
- The agent runs on the developer's live packages, so a host update changes its environment. A Hyprland update without a matching plugin build stops `pacman -Syu` with a dependency error until Omarchy ships one.
- `no_focus` against real input and a Takeover through the `keek` submap haven't run with a person yet. #8 and #9 prove them, so a problem there surfaces after the sandbox is built on.
- CPU encoding of the recording competes with the developer's own work on the same machine.
- Two fixed slots assume a wide monitor. On a 1920-pixel display each Session Window is under 960 pixels wide.
- The 116 ms constant and 150 ms guard come from one machine, with a hidden window at 15 fps and a scripted Takeover. A busier machine or a visible window could move them, and a cut that lands late would leave Takeover footage in the Verification Video. #9 rechecks this live.
- A Takeover cut removes time, not what is still on screen at Done. A code left visible in a field shows in the video after the title card.
- During a Takeover the nested desktop types as `us`, whatever the developer's layout is, because the plugin requires it. A password typed for another layout fails to sign in.
- The editor seeks by time. On a recording with long gaps between frames, which a visible Session Window may produce, a segment starting inside a gap opens on the frame after the gap. Recordings with a hidden window had gaps of at most 71 ms. #8 shows a window to a person, so recheck this there.
- keek carries an ffmpeg editor and a credential scrub until Cua releases their replacements, and a piece Cua declines stays keek's to carry.
- The Verifier split and the retry cap hold by instruction only. A coding agent that ignores them can verify its own change.
