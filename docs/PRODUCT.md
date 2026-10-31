# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

keek has no web or mobile surface. Its interface is native to the Omarchy Linux desktop: a nested Hyprland window per Agent Session, a gtk4-layer-shell Session Footer beneath it, a Verdict panel over the session's last frame, and Omarchy notifications. `web` is recorded only because it is the non-mobile value this record accepts.

## Users

A developer on Omarchy who codes alongside a coding agent (Claude Code or another MCP-capable harness). They keep working in their editor while one or two agents verify changes in the background, glance at the agents now and then, and step in only when an agent is stuck on something it cannot do itself, such as a 2FA prompt, a captcha or an OS dialog.

The second audience is the reviewer of the pull request, who never sees keek running and only meets its Evidence: the Verification Video and the Verdict Report attached to the PR.

## Product Purpose

keek lets a coding agent have its change verified by a fresh Verifier that uses the App Under Test like a person would, on a sandboxed nested Hyprland desktop inside the developer's own Omarchy session. It leaves Evidence a reviewer can attach to a pull request.

Success is a developer who trusts the Verdict without rerunning the check by hand, and a reviewer who can watch the verification in the time it takes to read the diff.

## Positioning

keek is an Omarchy layer on Cua Driver. Cua Driver performs and records Actions; keek runs the session on the developer's own desktop, with their own Hyprland, apps and Omarchy config, where they can watch it live and take control of it. The Verdict follows from a Verification Checklist the agent writes and the Verifier marks, so "Needs fixes" names the failed check instead of a bare fail.

## Operating Context

- Sessions show on a dedicated `keek` workspace, the Agents Workspace, with at most two Session Cards side by side (configurable). Each Session Card is the nested Hyprland window with its Session Footer joined beneath it.
- The Verifier drives keek only through its MCP server: `open_session`, `add_check`, `mark_check`, `request_help`, `close_session` and Cua Driver's forwarded tools.
- A Help Request turns that session's card orange and sends one desktop notification through `omarchy-notification-send`. The developer takes control, does the step, and hands control back with Done or Super+Esc.
- After a session ends, its card keeps the last frame, dimmed, with one Verdict panel over it until the developer dismisses it or a new session needs the slot.
- Evidence stays local under `~/Videos/keek`; the developer attaches it to the PR by hand.

## Capabilities and Constraints

- Omarchy is the only supported and tested host. keek uses Omarchy's helpers where they exist.
- Local only, MCP server only. No CI runners, no remote watching.
- A fresh Verifier subagent does each verification. After Needs fixes the coding agent may retry, up to three sessions per change.
- A session ends after 5 minutes without a tool call, as an Incomplete Session with no Verdict.
- `close_session` refuses while any Check is unmarked; keek derives the Verdict from the checklist.
- Verdicts are Pass, Needs fixes and Partly verified. Terms are defined in `docs/GLOSSARY.md`; decisions in `docs/adr/`.
- Long-running host code reacts to Hyprland events instead of polling, and costs no noticeable CPU while nobody looks at it.
- keek's surfaces take their colours from the active Omarchy theme. The reference comps use the Cumulus theme.

## Brand Commitments

- The name is `keek`, always lowercase.
- Voice is terse and technical. Sentence case, straight quotes, no exclamation marks, no emoji, no em dashes in prose. A control says what it does ("Take control", "Done", "Open video"). A state is one word or short phrase; its reason sits on the line beneath.
- Glossary terms are used exactly: Session Card, Session Footer, Verdict, Check, Help Request, Takeover. "Fail" is never a Verdict.
- keek must look like it belongs on Omarchy: Omarchy's bar, Hyprland's borders and rounding, JetBrains Mono, and the host theme's palette.

## Evidence on Hand

- Approved design canvas (private): https://claude.ai/artifact/LxCSK6QwgvUrNBKL8ftXzq
- Rendered reference comps of that canvas in `design/comps/`, kept outside git on the developer's machine.
- Architecture and viewer specs in `docs/design.md` and `docs/viewer.md`.
- No users, testimonials, benchmarks or release exist yet. Do not invent them.

## Product Principles

- Quiet until needed. keek never steals focus, pops over the editor or makes noise while nothing needs the developer.
- Control is explicit. Outside a Takeover the developer's input never reaches the agent's desktop, and the agent never acts while the developer is in control.
- Show the work, not the wait. Evidence fast-forwards idle stretches; the live view says what the agent did last and how long ago.
- The checklist is the verdict. Every outcome traces to marked Checks with short comments, so a reviewer sees what passed, what failed and what was skipped and why.
- Upstream first. Pieces Cua Driver should own are proposed to Cua Driver, and keek carries a stopgap only until a release has them; keek stays a thin Omarchy layer.

## Accessibility & Inclusion

- State is never carried by colour alone: Verdicts pair colour with an icon and a word, and checklist ticks differ in shape (filled, outlined with a dash, dim outline) as well as colour.
- Anything typed during a Takeover stays out of the Action Log and the Verification Video.
