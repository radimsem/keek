# keek

keek lets a coding agent have its change verified by a fresh Verifier on a sandboxed nested Hyprland desktop, and leaves Evidence for the pull request.

## Reference

- `GLOSSARY.md`: the only names for domain concepts, in code, tickets and copy. Reach for it before naming anything.
- `docs/design.md`: session, sandbox, tools, recording, Evidence.
- `docs/viewer.md`: Agents Workspace, Session Footer, Help Request, Takeover.
- `DESIGN.md` and `design/comps/`: any surface a person sees. `PRODUCT.md` holds the voice for copy.
- `docs/adr/`: before changing behaviour a decision records. A change that contradicts an ADR updates that ADR in the same pull request.
- `docs/facts.md`: what was checked about Cua Driver and Hyprland, with versions.

## Delivering a ticket

A ticket's plan is a proposal: its module names and flags can change, its acceptance criteria can't. If a criterion turns out wrong or the plan hits something the docs don't cover, stop and ask on the ticket.

1. Branch from `main` as `type/<issue>-<slug>`. Done when `git status` is clean on the new branch.
2. Write the failing test for the first criterion, and watch it go red for the expected reason. Repeat through every criterion that can be tested without a desktop.
3. Run the ticket's Verification section, every step, and keep the real output. Done when each step shows its stated result. A step you can't run is reported as not run, with the reason.
4. Pass the gates: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.
5. Commit one coherent change at a time. Each subject is a Conventional Commit (`feat(session): refuse a second session per worktree`), with no body or trailers. Tests travel with their code.
6. Open a pull request titled as a valid commit subject. The body says why, has `Closes #<issue>`, and lists what you verified with its output. Done when every acceptance criterion is ticked or explained.

## Boundaries

- Core logic takes its clock, driver, compositor and process runner as injected traits, so `cargo test` runs without a desktop. Tests that need a real desktop are `#[ignore]` and named `live_*`.
- The Cua Driver binary is pinned. Changing the version is its own pull request, and rechecks `docs/facts.md`.
- Nothing is posted to `trycua/cua` until the developer approves the exact text.
- Tests use made-up credentials and throwaway accounts.
- The developer's own session stays undisturbed by tests: focus, active workspace, config and open windows end as they began.
