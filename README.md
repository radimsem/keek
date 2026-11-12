# keek

keek lets a coding agent have its change verified on a real desktop. A fresh Verifier agent uses your app like a person would, in a sandboxed nested Hyprland inside your own Omarchy session. It leaves a video and a report you can attach to the pull request.

You keep working while it runs. Each Agent Session shows on its own `keek` workspace, where you can watch it and take control when the agent is stuck on something like a 2FA prompt.

keek is an Omarchy layer on [Cua Driver](https://github.com/trycua/cua), which performs and records the agent's actions.

## Requirements

keek runs on Omarchy only. It needs:

- Omarchy on x86_64, with its own Hyprland package.
- Omarchy's `cua-hyprland-plugin` package, which is built for one exact Hyprland version.
- Cua Driver 0.38.0. keek refuses any other version.
- Rust 1.97 or newer, to build keek.
- A coding agent that speaks MCP, such as Claude Code or Codex.

## Install

1. Install the packages keek uses. `--needed` skips the ones you already have.

   ```sh
   sudo pacman -S --needed cua-hyprland-plugin bubblewrap passt mise ffmpeg wf-recorder git gtk4 gtk4-layer-shell
   ```

2. Install Cua Driver 0.38.0 with Cua's installer, pinned to that version.

   ```sh
   CUA_DRIVER_RS_VERSION=0.38.0 /bin/bash -c "$(curl -fsSL https://cua.ai/driver/install.sh)"
   ```

   keek reads the driver from `~/.cua-driver/packages/releases/0.38.0-x86_64-unknown-linux-gnu/cua-driver`.

3. Build and install keek.

   ```sh
   cargo install --git https://github.com/radimsem/keek --tag v0.1.0 --locked
   ```

4. Check the result.

   ```sh
   keek --version
   ```

## Add keek to your coding agent

keek is an MCP server. In Claude Code:

```sh
claude mcp add keek -- keek mcp
```

In another harness, register a stdio MCP server with the command `keek` and the argument `mcp`.

Then give the agent the setup skill. It tells the agent how to describe a project to keek. For Claude Code:

```sh
mkdir -p ~/.claude/skills/keek-setup
curl -fsSL https://raw.githubusercontent.com/radimsem/keek/v0.1.0/skills/keek-setup/SKILL.md \
  -o ~/.claude/skills/keek-setup/SKILL.md
```

## Set up a project

Each project needs a Project Profile once per machine. It holds the commands that install and start your app. Ask your agent to run the `keek-setup` skill in the repository. The agent reads the repository, writes the profile to `~/.config/keek/projects/<project>.toml`, and proves that the app starts.

Two things are yours to add, beside the profile:

- Test Credentials, in `~/.config/keek/projects/<project>.env`. Use test-only accounts. The agent sees their names, never their values.
- Test Files, in `~/.config/keek/projects/<project>.files/`. These are sample inputs the app needs, such as a file to upload.

## Verify a change

Ask your agent to verify its change with keek. The agent writes a Verification Checklist and hands it to a fresh Verifier. The Verifier opens an Agent Session, uses the app, and marks each Check.

The session appears on the `keek` workspace. When the Verifier needs you, its card turns orange and you get one notification. Take control, do the step, and hand control back with Done or Super+Esc. The video blurs the Takeover, and what you type stays out of the Action Log. Clear any code left on screen before you hand back, because it shows unblurred afterwards.

The Verdict follows from the checklist. It is Pass, Needs fixes or Partly verified.

## Evidence

keek files Evidence under `~/Videos/keek/<project>/<date>/`. Each session leaves:

- a Verification Video, ending in `_verification.mp4`, with idle stretches fast-forwarded;
- a Verdict Report, ending in `_report.md`, with every Check and its comment.

Attach the video to the pull request by hand.

## Known limitations

- An X11-only app can't run, because Xwayland is off in the sandbox.
- Evidence stays on your machine. keek uploads nothing.
- Two Agent Sessions can be open at once by default, one per git worktree.
- During a Takeover the nested desktop types with the `us` layout, whatever your own layout is.
- A Hyprland update needs a matching `cua-hyprland-plugin` build from Omarchy before keek works again.
- keek edits the Verification Video with its own ffmpeg editor until Cua Driver can fast-forward idle time ([#11](https://github.com/radimsem/keek/issues/11)).
- keek scrubs Test Credentials from Evidence itself until it runs a Cua Driver that masks typed secrets ([#12](https://github.com/radimsem/keek/issues/12)).

Cua Driver is still pre-release, and its Hyprland support is experimental.

## Documentation

- [`docs/design.md`](docs/design.md) covers the session, sandbox, tools, recording and Evidence.
- [`docs/viewer.md`](docs/viewer.md) covers the Agents Workspace, Help Requests and Takeovers.
- [`docs/GLOSSARY.md`](docs/GLOSSARY.md) defines every term used here.
- [`docs/adr/`](docs/adr/) records the decisions behind the design.

## License

[MIT](LICENSE)
