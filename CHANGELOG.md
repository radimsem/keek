# Changelog

What changed in each keek release, for a person who uses it. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

### Fixed

- Recordings no longer start black beneath Omarchy's startup cover.
- `open_session` waits for Omarchy's startup cover to clear and refuses with a reason if it stays.
- A sandbox that fails to start reports its own log lines in the refusal.
- A missing `~/.config/hypr` or `~/.config/omarchy` is named in the refusal.
- An overlong Hyprland socket path is reported with the path and its length.
- A failed `open_session` leaves no session folder behind.
- `request_help` says the developer pressed Done and handed control back, and states the screenshot's scale.

### Changed

- The Verifier is told to mark each Check when it observes it, so the report and video point to that moment.
- The setup skill explains how to prove a browser app starts when the probe has no desktop.

## 0.1.0 - 2026-11-14

The first release. keek lets a coding agent have its change verified on a real desktop. A fresh Verifier agent uses your app like a person would, in a sandboxed nested Hyprland inside your own Omarchy session. It leaves a video and a report you can attach to the pull request.

### What it does

- Runs as a stdio MCP server, `keek mcp`, for a coding agent such as Claude Code, Codex, OpenCode, Cursor or Antigravity CLI.
- Shows each Agent Session on the `keek` workspace. The nested desktop's windows and input stay inside it.
- Lets you take control when the Verifier is stuck on something like a 2FA prompt, and blurs that Takeover in the video.
- Derives the Verdict from the Verification Checklist: Pass, Needs fixes or Partly verified.
- Files a Verification Video and a Verdict Report under `~/Videos/keek/<project>/<date>/`.
- Comes with the `keek-setup` skill, which writes a Project Profile and proves that your app starts.

### Requirements

keek runs on Omarchy only. It needs:

- Omarchy on x86_64, with its own Hyprland package.
- Omarchy's `cua-hyprland-plugin` package, which is built for one exact Hyprland version.
- Cua Driver 0.38.0. keek refuses any other version.
- Rust 1.97 or newer, to build keek.
- A coding agent that speaks MCP.

The [README](https://github.com/radimsem/keek/blob/v0.1.0/README.md#install) has the install steps.

### Known limitations

- An X11-only app can't run, because Xwayland is off in the sandbox.
- Evidence stays on your machine. keek uploads nothing, and you attach the video to the pull request by hand.
- Two Agent Sessions can be open at once by default, one per git worktree.
- During a Takeover the nested desktop types with the `us` layout, whatever your own layout is.
- A Hyprland update needs a matching `cua-hyprland-plugin` build from Omarchy before keek works again.
- keek edits the Verification Video with its own ffmpeg editor until Cua Driver can fast-forward idle time ([#11](https://github.com/radimsem/keek/issues/11)).
- keek scrubs Test Credentials from Evidence itself until it runs a Cua Driver that masks typed secrets ([#12](https://github.com/radimsem/keek/issues/12)).

Cua Driver is still pre-release, and its Hyprland support is experimental.
