# Changelog

What changed in each keek release, for a person who uses it. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

## 0.3.0 - 2026-11-26

### Added

- Add the omarchy-keek Bar Widget to see open sessions in the Omarchy bar, spot a Help Request at a glance, and switch to the Agents Workspace and back.
- keek's notifications show the keek logo.
- The Verifier can hand routine steps to a fast Decision Model, such as TypeSafe's Jev, and keek acts only when the model is confident.

### Changed

- Switch sessions with Super+F1 to Super+F4, so Super+number switches workspaces on the Agents Workspace too. Remove the Super+1 to Super+4 snippet from your Hyprland config and add the new one from the viewer docs.

### Fixed

- A rail card shows its session's shortcut only once you have added the bind, so it never names a key that does nothing.
- The Check in progress is now a blue-outlined tick with a dot in its centre, the same size as the other ticks, in place of a ring that sat off-centre.

## 0.2.0 - 2026-11-24

### Added

- Step away from a Takeover with the footer button or Super+Tab to use your own desktop, then click the session to resume. The Verifier keeps waiting until you press Done. Add the Super+Tab bind from the updated submap snippet in the viewer docs to use the shortcut.
- Paste text copied on your own desktop into the session during a Takeover, such as a 2FA code from your password manager. keek clears the session's clipboard when you press Done, and nothing copied in the session reaches yours.
- Watch up to four sessions at once: one in the focus slot and the others as rail cards with a still, their state and their Checks. Hover a rail card to see its goal and Checks, and click it to bring that session into focus. Add the Super+1 to Super+4 binds from the viewer docs to switch with the keyboard.

### Fixed

- The Session Footer's state text keeps its left edge when it changes, and long text is cut on the right.
- The Verifier is told when you press Done and the screen looks unchanged, so it asks for help again or skips the Check instead of treating the Help Request as answered. The Session Footer and the Verdict Report say the Help Request went unanswered.

### Changed

- A Session Window with no `window` in its Project Profile now fills its slot. The Verification Video size follows the developer's monitor, and the Verdict panel stays within the window.
- The Verifier is reminded every 8 calls to mark the Checks it has already observed, and the Session Footer rings the Check in progress, so the ticks fill in during the session.
- Four sessions can be open at once by default, up from two. Every Session Window opens at the focus slot's size, so the Verification Video is wider than before.
- Activating a Help Request notification now also brings that session into focus.

## 0.1.1 - 2026-11-15

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
