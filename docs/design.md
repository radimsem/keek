# keek design

keek lets a coding agent have its change verified by a fresh Verifier that uses the App Under Test like a person would, on a sandboxed nested Hyprland desktop inside the developer's own Omarchy session, and leaves Evidence a reviewer can attach to a pull request. Terms are defined in [`GLOSSARY.md`](GLOSSARY.md); decisions that are hard to reverse are in [`docs/adr/`](adr/).

The model is Warp's factories pipeline: the agent's verification is recorded, and the stretches where it is only reasoning are squeezed out of what the reviewer watches.

## Scope

v1:

- Your own projects (web apps in a browser, native Wayland GUI apps), run on the developer's own machine. An X11-only app can't run, because Xwayland is off in the sandbox.
- Omarchy is the only supported and tested host. An Agent Session runs the host's own Hyprland, apps and Omarchy config, nested inside the developer's session.
- One Agent Session per git worktree, at most four open at once (`session_cap`, configurable). Each open session takes a place on the Agents Workspace: the focus slot or the rail ([`viewer.md`](viewer.md#focus-slot-and-rail)).
- Evidence stays local, under `~/Videos/keek`. The agent prints its paths; the developer attaches the Verification Video to the PR by hand.

Later:

- Testing Omarchy itself: a Project Profile setting that mounts an Omarchy worktree's `bin/`, `config/` and `shell/` into the Agent Session in place of the developer's own. The design keeps this seam open and does nothing else for it in v1.
- Upload delivery (storage you configure, linked from the PR body).

Out of scope:

- Omarchy core integration until keek is stable ([ADR 0002](adr/0002-standalone-not-omarchy-core.md)).
- CI runners. keek verifies on the developer's machine, where the live view and Takeover matter; Cua's `e2e-test-agent` covers CI.

## Built on Cua

keek is an Omarchy layer on Cua Driver ([ADR 0009](adr/0009-shared-pieces-go-to-cua-driver.md)). Cua Driver performs Actions and records them. keek adds the sandboxed nested session on the developer's own Omarchy, the viewer and Takeover, its MCP tools and Evidence filed by Project.

keek runs stock Cua Driver 0.38.0 and doesn't wait for upstream changes ([ADR 0013](adr/0013-keek-times-the-session-on-stock-cua-driver.md)). Four pieces still belong in Cua Driver. Until a Cua release carries them:

| Piece | Today in Cua Driver 0.38.0 | Upstream | keek meanwhile |
| --- | --- | --- | --- |
| Hyprland video aligned with the Action Log | Turn times read about 1.45 s behind the video. | [#6210](https://github.com/trycua/cua/issues/6210), [PR #6239](https://github.com/trycua/cua/pull/6239) | The Session Timeline times Actions itself. This stays after the fix. |
| Idle Fast-forward in `cua-driver recording render` | `render` zooms around each Action; it has no Fast-forward. | [#6211](https://github.com/trycua/cua/issues/6211) | Its own ffmpeg editor (`stopgap`), removed once a release carries #6211 and the #6239 fix. |
| Masking typed secrets in `action.json` | Typed text is stored in plain text. | [#6250](https://github.com/trycua/cua/issues/6250) | Scrubs Test Credentials when filing Evidence (`stopgap`), removed once a release carries masking. |
| Resolving pid from window_id and re-reading a capture after a refusal | click and get_window_state refuse with the texts in facts.md | draft issue, not yet filed | Appends a window list or a get_window_state instruction under Cua's refusal (`stopgap`), removed once a release carries it. |

Issue and pull request text is approved by the developer before it is posted.

## Isolation

An Agent Session is a nested Hyprland in a bubblewrap sandbox under the developer's uid ([ADR 0006](adr/0006-nested-hyprland-in-a-sandbox-under-your-uid.md)). The coding agent already runs as the developer with a shell, so the sandbox keeps the agent's desktop from disturbing the developer's. It is not a wall against a hostile agent.

keek trusts the repository it is given. It runs git on the host, in the folder the caller passes, against that repository's own config, and in each initialized submodule's folder under that submodule's own config. Those runs turn off `core.fsmonitor`, `core.hooksPath` and lazy fetch, and `open_session` refuses a `core.worktree` that moves the top level or an initialized submodule's. A hostile repository is out of scope for v1. Risks lists the routes that stay open.

| Resource | In an Agent Session |
| --- | --- |
| Screen, input, focus | Its own nested Hyprland. The agent's input arrives on the Cua plugin's seat. The host shows the session as a Session Window that ignores the developer until Take control. |
| Host compositor | One connection, passed in as `WAYLAND_SOCKET` and created through a `wp_security_context_v1` socket, so it can't use screencopy or virtual input against the host. The daemon ends the session if this connection opens another window or its Session Window changes class. It checks on `openwindow` and every second; the window can exist until detection and sandbox termination. |
| Desktop services | Its own D-Bus session bus and accessibility bus, and its own Omarchy shell, notifications and clipboard inside the nested desktop. The shell hides its bar through a session-owned `toggles` folder holding `bar-off`; notifications still show, though one sent in the first seconds of `start` did not ([`facts.md`](facts.md#bare-nested-desktop-results)). The nested desktop binds no host clipboard or primary selection, so neither crosses on its own; this rests on source reading. During a Takeover keek carries the host clipboard's plain text in one way: at Take control and every Resume the daemon reads it with `wl-paste` and writes it as a length-prefixed frame to `clipboard.fifo` in the session folder, where `/keek/nested/clipboard.sh` hands it to `wl-copy --sensitive` on standard input. On Done the daemon sends a clear frame before it releases the held call. The FIFO is removed when the sandbox stops, and by recovery, so a kept session folder holds no pipe; Evidence never copies it. The text is in no command line, environment variable, Cua call, Action Log or Session Timeline, and nothing goes from the session to the host. The live checks have not run ([`facts.md`](facts.md#takeover-clipboard-results)). No `DISPLAY`, the host's or its own: Xwayland is off. |
| Home and config | A throwaway HOME. The developer's Hyprland and Omarchy config are mounted read-only, with keek's overrides on top: scale 1, the Cua plugin and the `us` keymap it requires, Xwayland off, no idle lock, bar hidden, gaps, borders and rounding at 0, and windows opaque. The session binds its own copy of Omarchy's toggles read-only. |
| Session folder | `~/.cache/keek/sessions/<session>/`, bind-mounted writable, so Cua Driver records where the host can render and file. |
| Worktree | A throwaway write layer over a tree of tracked and untracked files, over the Project Layer, in that order from top to bottom. The sandbox mounts it at `/keek/worktree`, so a Project Layer works from any worktree. `.git` is absent, so `git` does not work inside. The tree wins over install writes to tracked paths so a Verifier sees the agent's change. Git-ignored files (`.env*`, local builds, `node_modules`) are hidden unless the install command creates them. For a sample input that git ignores, use Test Files. |
| Test Files | A read-only bind of `~/.config/keek/projects/<project>.files/` at `/keek/files` in the Agent Session and the `prepare_project` probe, absent from install. |
| Network | A private network namespace through pasta: outbound internet and its own localhost. Before an Agent Session's sandbox starts, keek makes `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`, `100.64.0.0/10`, `169.254.0.0/16`, `fc00::/7` and `fe80::/10` unreachable using policy rules that sandboxed code cannot remove. The DNS forwarder at `169.254.1.1` is excepted, on every port. If `ip` is missing or any rule fails, including on a host with IPv6 turned off, keek refuses the Agent Session and names the failing command in `sandbox.err`. A Project reaches a host loopback service through a forwarded port; a LAN service can use a host loopback tunnel and a forwarded port. LAN neighbours with public addresses and special ranges outside this list remain reachable. The install phase keeps LAN access so an install command can reach a private package mirror. |
| Shared with the host | Kernel, uid, installed packages and toolchains, GPU, and the Toolchain Store, read-only. |

## Components

keek is one Rust binary ([ADR 0014](adr/0014-keek-is-one-rust-binary.md)).

- **Nested Hyprland**: the host's installed Hyprland, started inside the sandbox. Its output always takes the size of its window on the host, so keek floats that window at a fixed size ([ADR 0007](adr/0007-nested-window-is-the-session-window.md)).
- **cua-hyprland-plugin**: from Omarchy's package repository, which builds it against exact Hyprland and aquamarine versions. `open_session` checks that the plugin loaded into the nested Hyprland and refuses otherwise, naming both versions.
- **Project Profile**: `~/.config/keek/projects/<project>.toml`, one per Project on each machine ([ADR 0016](adr/0016-a-project-profile-not-a-setup-script-or-dockerfile.md)). A coding agent writes it with `set_project_profile`; the developer rarely edits it by hand. keek does no detection of its own: the agent reads the repository, such as `.nvmrc`, lockfiles and `package.json`, and writes explicit values. It holds two shell commands that keek runs with `/usr/bin/bash -c` from the worktree root inside the sandbox, in two phases *(my call: the two phases)*: `install` (cached as the Project Layer; optional, and without it the Project Layer is empty) and `start` (starts the App Under Test; required). Helper scripts they call are ordinary repository files. Both phases get `KEEK_PROJECT`, set to the Project name, the pinned toolchains on `PATH`, and the `[env]` table: non-secret values such as `APP_ENV`, which are not masked and which the Verifier can see. `[env]` refuses names keek sets itself, such as `HOME`, `PATH`, `KEEK_*`, `XDG_*` and `WAYLAND_*`, and a Test Credential with the same name as an `[env]` entry wins. The profile also holds `ports`, the host ports to forward into both install and session sandboxes, and `window`, the Session Window size. Without it, the window fills the focus slot above the Session Footer, whichever place the session starts in, so the Verification Video size follows the developer's monitor. One ordinary window fills the Session Window at 0,0. A `window` smaller than the focus slot is centred on black at its true size, and `open_session` refuses one larger than the focus slot. The agent's desktop and the Verification Video keep the Session Window size ([`viewer.md`](viewer.md#agents-workspace)). `open_session` refuses a Project with no profile and names `set_project_profile`. Unknown keys, an empty `start`, zero ports and zero window dimensions refuse `open_session` with the file path. keek no longer reads `.keek/setup.sh`. A non-zero `start` does not refuse an Agent Session; its output goes to `start.log`. After the driver handshake and plugin check, keek resizes the Session Window by the connect-exec pid before recording starts.
- **Project Layer**: the `install` phase's writes, cached under `~/.cache/keek/projects/<project>/` and keyed by SHA-256 of the length-framed install command, each toolchain's resolved version (so `node = "24"` rebuilds when it resolves to a newer 24.x), the sorted `[env]` names and values, and the sorted tracked lockfile paths and contents read from disk, including those in initialized submodules. Uncommitted lockfile edits change the key; untracked lockfiles and changes to `start`, `ports` or `window` do not. An exclusive lock for the Project serializes check and build. A build starts from the copy of the worktree files, writes into `.build/upper`, then renames that upper directory to the key on success. keek records each use in `.used/<key>` under the Project's lock and keeps the Project's two most recently used Project Layers. Each time keek checks or builds one, it removes the rest, skipping any that an open Agent Session or a running `prepare_project` mounts. Writes outside the worktree, including HOME and `/tmp`, are discarded. HOME is replaced, so toolchains under the real HOME, such as mise, rustup and nvm, are not visible; a Project reaches another version through the Toolchain Store. Failed or timed-out installs remove `.build` and refuse `open_session` with the last 4 KiB of the host-owned `install.log`; install has a separate 20 minute limit before the desktop handshake.
- **Toolchain Store**: `~/.cache/keek/toolchains/`, where keek installs the Project Profile's `toolchain` pins with `/usr/bin/mise` on the host, before the install sandbox starts. `toolchain` takes `node`, `rust`, `python`, `pnpm` and `yarn`, each a version mise reads, such as `"24"` or `"24.11.1"`; without a pin the sandbox's `/usr/bin` toolchain is used. keek runs mise with an empty environment, apart from the host's proxy and CA variables, and points every folder mise reads or writes into the store, including its config, its system config and the rustup and cargo homes, so the developer's `~/.config/mise`, `~/.local/share/mise`, `~/.rustup` and `~/.cargo` are neither read nor changed ([`facts.md`](facts.md#toolchain-store-results)). One lock serializes mise across Projects. A pin the store already holds is reused without a download: `mise where` reports the newest installed version matching it, and only a miss runs `mise install`, whose output goes to `toolchain.log` in the session folder, or in `~/.cache/keek/prepare/<project>/` for `prepare_project`. The store is mounted read-only at the same path in install and session sandboxes, with each version's bin folders ahead of the rest of `PATH`. A rust pin also sets `RUSTUP_HOME` into the store and `RUSTUP_TOOLCHAIN` to the version; `CARGO_HOME` stays unset, so cargo's registry and other writable caches, like pnpm's store, go to the sandbox's throwaway HOME.
- **Worktree files**: `open_session` refuses a repository whose git top level is not the folder holding its `.git`, which is what a `core.worktree` setting does, so a repository's config can't choose the host folder that is copied. `git ls-files -co --exclude-standard -z` supplies raw paths, sorted and deduplicated. Regular files use reflink, then copy if reflink is unavailable, and symlinks retain their target. Each initialized submodule (a gitlink whose folder holds `.git`) is listed the same way from inside it after the same top-level check, so its `core.worktree` can't add another host folder's files. A submodule's files follow its own ignore rules, so the superproject's `.gitignore` does not hide an untracked file inside it. An uninitialized submodule, a submodule git can't open (logged as a warning) and a nested repository that isn't a submodule stay empty directories. Missing files and special files are skipped. The tree lives at `~/.cache/keek/trees/<session>/`, outside the writable session folder so the sandbox cannot modify an overlay lower layer. Closing or refusing an Agent Session removes its tree. When it starts, the daemon removes every tree under `~/.cache/keek/trees/` before it finishes Incomplete Sessions, which need only the session folder.
- **Test Credentials**: `~/.config/keek/projects/<project>.env`, injected into the sandbox as environment variables for the App Under Test. Their values reach the sandbox through connect-exec's environment and one arguments file descriptor, so no process's command line carries them. The Verifier gets only their names from `open_session`. It types `${KEEK_TEST_PASSWORD}`, and keek substitutes the value before forwarding the call and masks secret values in what comes back. Public values, such as a test username, stay readable. Cua Driver still records the value, so keek scrubs every secret credential value from the Action Log's text files when it files Evidence, and removes the session's cache folder once filed. Screenshots can't be scrubbed, which is why Test Credentials are test-only. The env file has one `NAME=value` per line, with blank lines and comments skipped and an optional `export ` prefix. A comment line of exactly `# keek: public NAME`, which the developer adds by hand, marks that name public; a marker for an absent name is ignored. Names use `[A-Za-z_][A-Za-z0-9_]*`. Matching single or double outer quotes are removed literally without escape processing; unquoted values are trimmed. Duplicate names and secret values shorter than 6 characters refuse opening. A mode readable by group or others warns to run `chmod 600 <path>`. For secret values, masking replaces raw, JSON-escaped (once or twice), and percent-encoded forms with `[masked]`. Filing scrubs UTF-8 files recursively inside direct `turn-*` folders and `timeline.json`, leaving binary files unchanged. A scrub failure, including a file over 64 MiB, withholds the Action Log, returns `actions_error`, and retains the session folder for the developer to check and delete.
- **Test Files**: `~/.config/keek/projects/<project>.files/` ([ADR 0017](adr/0017-test-files-live-beside-the-project-profile.md)), placed there by the developer and read-only at `/keek/files` in each Agent Session and the `prepare_project` probe. keek lists regular files recursively, including hidden files, as sorted relative paths, skips symlinks and special files, and returns at most 100 names with `more_files` counting the rest. A missing folder gives an empty list. A symlinked folder is refused. Test Files stay outside the Project Layer key and the install sandbox. No keek tool writes there. Their names appear in the Verdict Report's Project Profile section and must be safe to appear in Evidence.
- **Host MCP server**: what keek ships to the coding agent (`claude mcp add keek -- keek mcp`, or the equivalent in another harness; [ADR 0011](adr/0011-mcp-server-only.md)), with one setup skill. Its instructions describe the Verifier protocol. It is a thin client of the keek daemon and starts one if none is running. The Hello handshake orders builds by semver version, then by the running executable's modification time from `/proc/self/exe`, read once per process. A missing build is older than any known build of the same version. If the client cannot read its own build, it logs one warning and compares versions only. An older daemon exits on request when no Agent Session is open; otherwise the client refuses and names both versions and, when equal, both builds as UTC timestamps. A client also refuses a newer daemon.
- **Scripted trials**: `claude mcp add keek -- keek mcp` was blocked by Claude Code's auto mode as self-modification, so a scripted `claude -p` run gets keek per run instead. With `--mcp-config <configs...>` ("Load MCP servers from JSON files or strings (space-separated)"), pass a file containing `{"mcpServers":{"keek":{"command":"<path to keek>","args":["mcp"]}}}`. Add `--strict-mcp-config` ("Only use MCP servers from --mcp-config, ignoring all other MCP configurations") so no other server loads. `--allowedTools` ("Comma or space-separated list of tool names to allow ...") only pre-approves tools and removes none, so in the third trial the Verifier could still reach the host shell. Two flags take host tools away: `--disallowedTools <tools...>` ("Comma or space-separated list of tool names to deny ...") and `--tools <tools...>` ("Specify the list of available tools from the built-in set. Use "" to disable all tools ..."). Neither was run in the trial. The flag descriptions were checked against `claude --help` for Claude Code 2.1.319 on 2026-11-09.
- **Setup skill**: `skills/keek-setup/SKILL.md`, which tells the coding agent how to read a repository, choose `install`, `start` and `toolchain`, put non-secret values in `[env]` and secrets in Test Credentials, and call `set_project_profile` and then `prepare_project` until `start` runs, or its log shows the server ready before a browser fails for the missing display. Its last step offers to add a Verification section to the repository's agent file, `AGENTS.md` when present, and writes it only when the developer agrees. The section names the repository's Projects, says to verify when a change alters what a person sees or does in the running app and to say so when skipping, points at this skill for a missing or invalid Project Profile, and says to put the Verdict in the pull request. The README shows the same section, and a test keeps the two identical. The Verifier protocol itself stays in the server's instructions ([ADR 0011](adr/0011-mcp-server-only.md)). The developer copies it into `~/.claude/skills` and `~/.agents/skills` by hand. A test parses its profile example and checks it uses every key of the tool's schema.
- **keek daemon**: one per user ([ADR 0015](adr/0015-one-per-user-daemon-owns-sessions.md)). It owns every open Agent Session, the session cap, the slots, the Session Footers and the connection to each Cua Driver, and it keeps the Session Timeline. It exits once no client, session or Session Card remains. `keek_status` reports its version, build as a UTC timestamp (absent if unreadable), pid and open Agent Session count.
- **Cua Driver** (inside the sandbox): performs Actions through the plugin's input seat and records the session ([ADR 0003](adr/0003-cua-driver-owns-input-and-recording.md)). It runs inside so that it sees the sandbox's accessibility bus and nested display. Its config lives in the sandbox's throwaway HOME, so keek's cursor settings never touch the developer's own. keek mounts the pinned binary from an explicit path, because `~/.local/bin` sits under the HOME the sandbox replaces, and turns the driver's telemetry off.
- **Renderer** (on the host): keek's ffmpeg editor, cutting from the Session Timeline, plus keek's blurred, compressed Takeover footage. Cua Driver's `recording render` replaces the editor once a release carries Fast-forward.

A Project Profile uses this format; only `start` is required, and a supplied window has two non-zero dimensions:

```toml
install = "npm ci"
start = "npm run dev -- --host 127.0.0.1"
ports = [5432, 6379]

[toolchain]
node = "24"
pnpm = "9"

[env]
APP_ENV = "test"

[window]
width = 390
height = 844
```

`open_session` always returns a `warnings` array. A missing Test Credentials file adds a warning naming the Project and path.

## Viewer

The developer watches Agent Sessions and takes them over on the Agents Workspace, where each Session Window is the nested Hyprland's own window and its Session Footer is joined beneath it; see [`viewer.md`](viewer.md).

## Verifier protocol

The coding agent never drives an Agent Session itself ([ADR 0010](adr/0010-a-fresh-verifier-checks-each-change.md)). keek's server instructions tell it to:

1. Write the goal as a short list of acceptance checks a person could verify on screen ("Signing in with the test account lands on the dashboard"). They become the Verification Checklist.
2. Start a fresh subagent, the Verifier, with only the goal and its checks, the Project name and the worktree. No diff and no notes on what changed.
3. Read the Verifier's Verdict and Evidence paths. A Verdict is final.
4. After Needs fixes, read the failed Checks and their comments, fix the change and start a new Verifier, up to 3 Agent Sessions per change. Then stop and report to the developer with every session's Evidence. After Partly verified, tell the developer which Checks were skipped and why.

To wait for the screen to change, call `get_window_state` again rather than sleeping in a shell.

`open_session` returns `credentials`, the names of the Project's Test Credentials. To use one, write `${NAME}` in a tool argument, including inside `run_steps`. keek substitutes the value before forwarding and masks secret values in results, so the Verifier never sees a secret value. Public values, such as a test username, stay readable in results and Evidence. An unknown name is refused. In `run_script`, substitution is plain text inside JavaScript, so prefer `run_steps` for a value with quotes or backslashes.

`open_session` also returns `files`, the names of the Project's Test Files: sample inputs, such as a document to upload, that the developer placed outside the repository. They are read-only at `/keek/files/<name>`; in a file dialog press Ctrl+L and type that path. `more_files` counts names left off a long list. Git-ignored files are absent from `/keek/worktree`, so a path to one fails: use a Test File, or mark the Check skipped and say why.

Call `mark_check` for each Check as soon as you observe it, rather than marking all Checks at the end, because its mark time is where the Verdict Report and Verification Video point.

Judge each Check only from what the Agent Session shows. Don't read the worktree's source or run host commands to explain a result.

`type_text` into a password field returns `effect: unverifiable`, and the field can't be read back. Click the field and clear it with ctrl+a before typing, so a retype doesn't double the password.

keek can't tell which agent calls its tools, so these steps hold by instruction only.

## Agent interface

The host MCP server exposes these to the Verifier. MCP calls carry no caller identity and one coding agent can run two Verifiers, so every tool but `open_session` takes the session's name as a required `session` argument:

- `open_session(project, worktree, goal, checks)`: refuses if this worktree already has an open Agent Session or the cap is reached. Any Project name is accepted, and one repository can hold several Projects ([ADR 0008](adr/0008-agent-names-the-project.md)). It records the worktree's branch, commit and whether it has uncommitted changes. It then:
  1. builds or reuses the Project Layer
  2. starts the sandbox and nested Hyprland in a free place, the focus slot first
  3. injects Test Credentials and starts the App Under Test
  4. starts recording and the Session Timeline

  While it runs it sends a progress notification with a message at each phase (Building Project Layer, Starting desktop, Starting App Under Test), when the client sent a progress token, and Installing toolchains inside the first phase when the Project Profile pins one. During an install it also sends the newest finished line of `install.log`, prefixed `install: `, at most every 10 seconds, cut to 200 characters and masked like any result. Claude Code aborts a local MCP call after 30 minutes with no response or progress, and a cold install can take 20 minutes.

  It returns once the desktop is ready, with the session's name (`myapp_14-03-22`), each Check's id and text, the names of its Test Credentials, the Test File names in `files`, and `more_files`, the count left off a list longer than 100. It refuses a Project with no Project Profile, naming the `keek-setup` skill and telling a Verifier to report back to the coding agent, and warns when the Project has no Test Credentials, which is how a misspelled name shows up.
- Cua Driver's tools (`get_window_state`, `click`, `type_text` and the rest), forwarded with `session` added to their schema and stripped before the call. These are not forwarded, because each would undo something keek owns:
  - Recording is always on: `start_recording`, `stop_recording`, `get_recording_state`, `replay_trajectory`.
  - keek owns the driver session, whose end stops the recording: `start_session`, `end_session`, `escalate_session`.
  - keek owns the driver's setup: `set_config`, `check_for_update`, `install_extension`, `install_ffmpeg`, `set_agent_cursor_enabled`, `set_agent_cursor_motion`, `set_agent_cursor_theme`.

  A forwarded tool with a `delivery_mode` argument defaults to `foreground`, in its schema and when keek forwards the call. Cua's default is `background`, which Chromium refuses, and foreground input stays inside the nested desktop, so it can't disturb the developer. For the three refusals in `facts.md`, keek appends recovery guidance. For the two pid refusals it calls `list_windows` once, outside the Session Timeline and the Action Log.
- `request_help(session, reason)`: raises a Help Request and blocks until the developer finishes a Takeover; see [`viewer.md`](viewer.md#takeover). While it waits it sends progress notifications, because Claude Code aborts a local MCP call after 30 minutes with no response or progress. If the coding agent cancels the call, keek withdraws the Help Request; see [`viewer.md`](viewer.md#help-request).
- `add_check(session, text)`: adds a Check the Verifier found worth checking while testing. It is marked as added in the Verdict Report and on the card.
- `mark_check(session, check, outcome, comment)`: marks a Check passed, failed or skipped; failed and skipped need a one-line comment. A Check can be marked again until the session closes. keek pushes each mark to the Session Footer and logs it in the Session Timeline with its time.
  - **Reminder:** the Verifier is told to mark each Check as it observes it, and often marks them all just before `close_session`. keek counts the Cua calls it forwards since the last successful `mark_check` or `add_check`. Once 8 calls have passed with a Check unmarked, the next result gets one more line at the end of its content: `keek: Checks not marked yet: 1: <text>, 2: <text>. Call mark_check for each one you have already observed.` The count then restarts, so the line repeats every 8 calls and stops when no Check is unmarked. No call is refused, delayed or changed beyond that line. Each reminder is recorded in the Session Timeline, and the Session Footer shows it until the next call.
- `close_session(session, summary)`: refuses while a Check is unmarked and names the unmarked ones, and refuses a session with no Checks, which would otherwise Pass with nothing verified. It is decided ahead of queued calls, after any `add_check` and `mark_check` already queued. Once it passes, it stops a call in flight and refuses the queued calls; a refused close leaves them running. A held Help Request, a Takeover and its hand-back capture finish first. keek derives the Verdict from the Checks ([ADR 0012](adr/0012-verdict-follows-from-the-checklist.md)), stops recording, stops the sandbox, renders, files the Evidence and returns the Verdict, every Check with its outcome and comment, and the Evidence paths. If keek cannot stop the recording, it files an Incomplete Session in place of the Verdict, and the reply carries `incomplete`. If the session opens a window on the developer's desktop besides its Session Window, it is also incomplete, even if recording stops successfully. A sandbox close error after a good stop keeps the Verdict unless the session opened such a window. If the editor fails, keek still files the rest of the Evidence and returns `video_error`, and the Verdict Report says its Check times are positions in the Raw Recording.

The coding agent sets up a Project with these, which take no session ([ADR 0016](adr/0016-a-project-profile-not-a-setup-script-or-dockerfile.md)):

- `set_project_profile(project, profile, credentials?)`: its JSON schema is the Project Profile format. keek validates the profile and refuses an invalid one with the reason, writes the whole file with the previous one kept as `<project>.toml.bak`, and returns the written TOML. `credentials` holds an absolute host `source` path, such as the repository's `.env`, and the `names` to copy. keek copies those values into `<project>.env` with mode 600, keeps the file's other names, and keeps an existing value when the source lacks that name. It keeps every `# keek: public NAME` marker, which only the developer adds or removes by editing the file, so no agent can make a secret readable. The reply lists the names copied, kept and missing, never a value, and the developer fills in the missing ones.
- `prepare_project(project, worktree)`: checks the profile and worktree as `open_session` does, installs pinned toolchains, builds or reuses the Project Layer, then runs `start` in the session's sandbox, network and LAN block, with the Test Credentials and no desktop, recording or Session Window. keek stops `start` after 15 seconds, or sooner if it exits. The reply carries the toolchain versions, whether the Project Layer was `built` or `reused`, the last 4 KiB of `install.log` and `start.log`, masked, and `start` as `running` or `exited` with its status. When `start` launches a browser, `exited` is expected only if `start.log` shows the server ready, such as a listening line, before the browser fails for the missing display. Without a ready line, `exited` is a failure. It sends the same progress notifications as `open_session`. The logs stay in `~/.cache/keek/prepare/<project>/` until the next call. If the caller cancels, which Codex does after 5 minutes of active time, keek keeps building up to the 20 minute install limit, and a repeat `prepare_project` for the Project and worktree joins the build already running and gets its reply. One for another worktree of the Project is refused and names the worktree being prepared. An `open_session` waits on the Project Layer lock and then reuses the layer. Disconnecting the `keek mcp` that started the build cancels it, even when another call has joined it.

## Recording and editing

Recording starts in `open_session` and stops in `close_session`; the Verifier has no recording tool. Cua Driver's `start_recording` with `record_video:true` writes, per session:

- `recording.mp4`: the Raw Recording (wf-recorder on the nested output, CPU encoded). While the Session Window is hidden on the Agents Workspace, it records at the host's `misc:render_unfocused_fps`, 15 by default. keek leaves that setting alone. Cua Driver stops a recording after 300 s without a tool call, so while a Help Request or Takeover holds the Verifier's calls, keek sends Cua a read-only `get_recording_state` each minute ([`facts.md`](facts.md#recording-idle-results)). It enters neither the Session Timeline nor the Action Log.
- `turn-NNNNN/action.json` per Action: tool, arguments, result, click point, and `t_ms_from_session_start`.

Read-only calls (snapshots, accessibility trees) are not Actions and leave no turn, so the gaps between turns are where the Verifier was reasoning.

keek sets Cua's cursor for every session: action marks on (clicking, typing, scrolling), a ripple on each click, and the resting float off. Cua's default cursor floats while it rests, which would keep an idle screen from ever being still. keek draws no overlay of its own.

### Session Timeline

The first spike found the video running a steady 1.45 s behind Cua's turn times. keek doesn't use those times for anything on the video. It keeps its own Session Timeline at the MCP proxy ([ADR 0013](adr/0013-keek-times-the-session-on-stock-cua-driver.md)):

- keek reads a monotonic clock just before it forwards `start_recording`, and counts every later time from there, less a constant of 116 ms, because the video's clock sits that far behind this zero ([`facts.md`](facts.md#sandbox-spike-results)).
- Each forwarded call gets its start and end. A call is an Action when its tool isn't marked `readOnlyHint`, the same rule Cua uses for turns. One `run_steps` call is one active span, even though Cua logs a turn per step. A click's effect lands somewhere inside its call, up to 333 ms after its start, so nothing treats a call's start as the moment its effect shows.
- Check marks, Help Requests and Takeovers go on the same timeline. A Help Request records whether it was answered, unanswered or withdrawn. It is unanswered when the developer pressed Done and the screen looked unchanged from Take control ([`viewer.md`](viewer.md#takeover)).
- Each Check reminder goes on it with its time. It is not an Action and not in the Action Log, and the editor and the Verdict Report ignore it.

It is data only in v1: the editor and the Verdict Report read it, and nothing draws it. Cua's `t_ms_from_session_start` stays in the Action Log as recorded, so on 0.38.0 it reads about 1.45 s behind the video.

### Verification Video

The Verification Video ([ADR 0004](adr/0004-fast-forward-idle-instead-of-cutting.md)) comes from keek's ffmpeg editor until Cua Driver's `recording render` has the same edit:

1. Keep a window from 0.3 s before each Action's start to 1.5 s after its end.
2. Extend a window while the screen keeps changing (ffmpeg `freezedetect` at -55 dB over 0.5 s), so animations, loads and late toasts stay at real speed. A blinking caret, Cua's cursor and a small looping animation count as still ([`facts.md`](facts.md#still-screen-results)).
3. Merge overlapping windows. Everything left is an Idle Stretch.
4. Show each Idle Stretch as a Fast-forward of about 0.6 s.

A Help Request's wait is cut with nothing in its place, from the later of its start and the end of the last Action's window before it, because the wait is the developer's time, not the agent's ([ADR 0004](adr/0004-fast-forward-idle-instead-of-cutting.md)). Each Takeover shows as about 2 s of its own footage, blurred, with "You were in control" centred on top in navy or cream, whichever reads on the footage's mean brightness. The Verdict Report keeps the wait, the time in control and the reason. Each Takeover's guarded span reaches 150 ms past it on both sides, about twice the 65 ms the video varies against a page's own timestamps. These guards keep a frame with a 2FA code out of the unblurred footage. The same split stays when Cua's render replaces the editor, because that render knows nothing of Takeovers. The Raw Recording keeps the footage.

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
- `report.md` is the Verdict Report. Its header names the goal and the revision ("Verified fix/17-parser-bom at a1b2c3d + uncommitted changes"). It lists the Verification Checklist, each Check with its outcome, comment and timestamp in the Verification Video, then the Project Profile that built the environment, because a commit no longer says: its `install` and `start` commands, masked like any result, the resolved toolchain versions, the `[env]` names without their values, and the Test File names with a count of any left off the list. Then come the Verifier's summary and the Verdict. It states that its times come from the Session Timeline, not from `actions/`. It is ready to paste into a PR body after review.
- `index.md` gets one line per session, appended at close: date, time, branch, commit, Verdict, goal, and links to the files.
- keek never deletes Evidence.

Every file carries the Project and time in its name, so a Verification Video dragged into a PR still says where it came from.

## Abnormal end

An Agent Session can end without `close_session`: the Verifier finishes, crashes or runs out of context first, the coding agent's session exits, or the developer presses Ctrl-C. keek's MCP server belongs to the coding agent's session, so a Verifier ending ends no process keek can watch. Every Cua call passes through keek, so the daemon ends the session after 5 minutes without a tool call, outside a Help Request or Takeover, and when the `keek mcp` that opened it disconnects. It stops recording, stops the sandbox and renders anyway. If the daemon itself dies, the sandbox dies with it, and the next daemon to start finishes any session it finds unfiled the same way. The window rule keek installs for Session Windows matches every `aquamarine` window, including a developer's own nested Hyprland, so keek disables it when the last session ends, when the daemon stops on SIGTERM, and when the next daemon starts. Only when the last session ends does keek also move such a window off `keek` (`docs/viewer.md` has the details). The daemon also ends a session that opens a window on the developer's desktop besides its Session Window, including a second `aquamarine` window, or changes its Session Window's class. It compares host client pids with connect-exec's pid, checks on `openwindow` and polls every second for class changes. Detection and sandbox termination leave a short gap during which the window exists. The session is an Incomplete Session. Its files carry `incomplete` in place of a verdict, its Verdict Report says why it ended, lists the Checks marked so far and has no Verdict, and its Verification Video is labelled incomplete. While its Session Card remains, a call that names the session is refused with the reason and the Evidence folder, and the Verifier is told to open a new Agent Session.

An Agent Session can also end in a `close_session` that cannot stop the recording. It is an Incomplete Session because recording could not be stopped, unless it opens a window on the developer's desktop besides its Session Window, which takes precedence as the reason. A sandbox close error after a good stop keeps the Verdict unless the session opened such a window.

keek keeps an atomic, host-only journal for each open session under `$XDG_STATE_HOME/keek/sessions/`, because the sandbox can change files in the session folder and a retained folder may already be filed. After a daemon stops, the next daemon files the journaled session as incomplete and scrubs its Action Log only if the Project's Test Credentials file still has the same bytes, with both files absent counting as unchanged. If the file changed or was removed, keek withholds the Action Log and explains why in the Verdict Report.

If a Takeover or Help Request was still open when the daemon stopped, recovery cuts from its start to the end of the recording. The open Help Request counts as withdrawn.

The spikes' results and the facts this design rests on are in [`facts.md`](facts.md).

## Risks

- Cua's Hyprland support is experimental, and with no accessibility tree in Chromium the agent clicks by pixel. Scale 1 in the nested desktop keeps coordinates right (Cua issues #4944, #3061).
- The agent runs on the developer's live packages, so a host update changes its environment. A Hyprland update without a matching plugin build stops `pacman -Syu` with a dependency error until Omarchy ships one.
- `no_focus` against real input and a Takeover through the `keek` submap haven't run with a person yet. #8 and #9 prove them, so a problem there surfaces after the sandbox is built on.
- CPU encoding of the recording competes with the developer's own work on the same machine.
- Every Session Window opens at the focus slot's size, so on a 1920-pixel display it is about 1560 pixels wide, and a narrow monitor gives the focus slot less height to leave room for the rail row.
- A rail card's still is up to 3 s old, so a session that just changed looks behind until the next still.
- The 116 ms constant and 150 ms guard come from one machine, with a hidden window at 15 fps and a scripted Takeover. A busier machine or a visible window could move them, and a cut that lands late would leave Takeover footage in the Verification Video. #9 rechecks this live.
- What stays on screen after Done shows unblurred after the Takeover. A code left visible in a field still shows in the video.
- The -55 dB still-screen level comes from one session of the first trial on one machine. A page whose meaningful change is fainter than a caret blink, such as a faint progress bar, would be fast-forwarded.
- During a Takeover the nested desktop types as `us`, whatever the developer's layout is, because the plugin requires it. A password typed for another layout fails to sign in.
- The editor seeks by time. On a recording with long gaps between frames, which a visible Session Window may produce, a segment starting inside a gap opens on the frame after the gap. Recordings with a hidden window had gaps of at most 71 ms. #8 shows a window to a person, so recheck this there.
- keek carries an ffmpeg editor and a credential scrub until Cua releases their replacements, and a piece Cua declines stays keek's to carry.
- The Verifier split and the retry cap hold by instruction only. A coding agent that ignores them can verify its own change.
- Masking stops a Test Credential from leaking by accident, not a coding agent that tries to read one. The agent writes `start`, which gets the Test Credentials, so it can print a value base64-encoded and read it from `prepare_project`'s log tail. `set_project_profile` copies from any file the developer can read, which the agent can already read on the host. This is why Test Credentials are test-only. Closing it would mean a `prepare_project` with no Test Credentials or no log tail, which breaks an app that checks its environment at start and hides why `start` failed.
- A repository's config can name programs that keek's git runs don't turn off. `filter.<name>.clean` runs on a changed file during `git status`, and submodule recursion in `git status` runs git under each submodule's own config. keek also runs `ls-files` and `rev-parse` inside each initialized submodule itself, under the submodule's own config, with the same `-c core.fsmonitor=false -c core.hooksPath=/dev/null --no-lazy-fetch` flags as at the top level. Only `core.fsmonitor` was run: it fires on `git status` on git 2.56.0, and `-c core.fsmonitor=false` stops it. The other two routes come from git's documentation and haven't run. If scope changes, run `status` in a read-only, networkless bwrap that mounts the worktree and its `git rev-parse --git-common-dir`, or drop the dirty state.
- Codex 0.166 counts `tool_timeout_sec` (300 s by default) as active time and pauses it only while an MCP elicitation is outstanding (`codex-rs/rmcp-client/src/rmcp_client.rs`, `active_time_timeout`). Progress notifications don't extend it, so in Codex a call longer than 5 minutes of active time is cancelled. A cold `open_session` can pass that limit. `prepare_project` keeps building after a cancel, so the next call rejoins it, and an `open_session` after it reuses the Project Layer it built.
- In Codex, a Help Request must be answered within 5 minutes, or `request_help` is cancelled and keek withdraws the Help Request ([`viewer.md`](viewer.md#help-request)).
