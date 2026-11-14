# keek

keek lets a coding agent have its change verified by a fresh Verifier on a sandboxed nested Hyprland desktop, and leaves Evidence for the pull request.

## Reference

- `docs/GLOSSARY.md`: the only names for domain concepts, in code, tickets and copy. Reach for it before naming anything.
- `docs/design.md`: session, sandbox, tools, recording, Evidence.
- `docs/viewer.md`: Agents Workspace, Session Footer, Help Request, Takeover.
- `docs/DESIGN.md`: any surface a person sees. `docs/PRODUCT.md` holds the voice for copy.
- `design/comps/` and `.impeccable/`: design files kept outside git. A worktree or fresh clone lacks them, so read them from the primary checkout, or use the design canvas linked in `docs/DESIGN.md`.
- `docs/adr/`: before changing behaviour a decision records. A change that contradicts an ADR updates that ADR in the same pull request.
- `docs/facts.md`: what was checked about Cua Driver and Hyprland, with versions.
- `docs/releasing.md`: cutting a release. Tagging and publishing are the developer's to do or approve.

Skills and tools that default to a root `GLOSSARY.md`, `DESIGN.md` or `PRODUCT.md` use these `docs/` paths.

## Delivering a ticket

A ticket's plan is a proposal: its module names and flags can change, its acceptance criteria can't. If a criterion turns out wrong or the plan hits something the docs don't cover, stop and ask on the ticket.

1. Branch from `main` as `type/<issue>-<slug>`. Done when `git status` is clean on the new branch.
2. Write the failing test for the first criterion, and watch it go red for the expected reason. Repeat through every criterion that can be tested without a desktop.
3. Run the ticket's Verification section, every step, and keep the real output. Done when each step shows its stated result. A step you can't run is reported as not run, with the reason.
4. Pass the gates: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.
5. Commit one coherent change at a time, as Git below says.
6. Open a pull request titled as a valid commit subject. The body says why, has `Closes #<issue>`, and lists what you verified with its output. Done when every acceptance criterion is ticked or explained.

## Boundaries

- Core logic takes its clock, driver, compositor and process runner as injected traits, so `cargo test` runs without a desktop. Tests that need a real desktop are `#[ignore]` and named `live_*`.
- The Cua Driver binary is pinned. Changing the version is its own pull request, and rechecks `docs/facts.md`.
- Nothing is posted to `trycua/cua` until the developer approves the exact text.
- Tests use made-up credentials and throwaway accounts.
- Credentials, tokens, keys and `.env` contents stay out of output and commits.
- The developer's own session stays undisturbed by tests: focus, active workspace, config and open windows end as they began.

## Working

- Match process to risk: do clear, reversible changes directly, and plan when scope or design is uncertain.
- Make the smallest change that solves the task. Report unrelated problems and leave them in place.
- Before adding complexity for an unlikely race, failure or security case, ask first, with its failure mode, likelihood and cost.
- Keep source files under about 20 KB. Split by responsibility before crossing that.
- Report what passed, failed or wasn't run, with the real output.

## Git

- A commit is subject only: one `-m`, no body, no co-author or session trailers.
- The subject is `type(scope): description`, at most 72 characters, imperative, lowercase apart from identifiers, with no trailing period: `feat(session): refuse a second session per worktree`.
- Types are feat, fix, refactor, perf, test, docs, build, ci, chore, style and revert. Take the narrowest; chore is the last resort.
- The scope is a module or subsystem in kebab-case, left out for repo-wide changes. A breaking change puts `!` before the colon.
- Each commit is one coherent change that passes the gates alone. Tests travel with their code. Formatting and regenerated files get their own commit.
- Stage exact paths and read `git diff --staged` first. `Cargo.lock` changes only with a dependency change.
- `push --force`, `reset --hard`, `clean` and `branch -D` run only when asked.

## Changelog

- A change a person using keek would notice adds one line under `Unreleased` in `CHANGELOG.md`, in the same pull request, under Added, Changed, Fixed or Removed.
- Write the line for that person, in the voice of `docs/PRODUCT.md`: what they can now do or what stopped going wrong, with no module names.
- Refactors, tests, CI and internal docs add no line.

## Comments

A comment says what the code can't. Most declarations need none.

- Write reasons, invariants, units, side effects, surprising constraints and workarounds, in one short active line beside the code: `// The XDG spec says to ignore a variable that isn't an absolute path.`
- Cut paraphrases of a name, type or control flow, banners, process narration, TODOs with no issue, and invented rationale.
- keek publishes no library: `lib.rs` exists so `tests/` can reach its modules, and `pub` isn't public API. Write `///` or `# Errors` only for a contract the signature and the error type leave out. `///` on clap arguments stays, because it is the CLI's help text.
- Prefer a clearer name to a comment. A change removes the comments it makes stale.

## Rust

- One `thiserror` enum per module, one variant per failure mode. `anyhow` stays in `main.rs`, `mcp.rs` and tests.
- An error a person reads says what happened, why, and what to do next.
- Use newtypes for ids and units (`RequestId`, `Millis`), and enums for state in place of bools.
- Match enums keek owns exhaustively, with no `_` arm, so a new variant fails to compile where it matters.
- Accept `&str`, `&Path` and `&[T]`. Restructure a scope to satisfy the borrow checker, and keep `.clone()` for real copies.
- In async code, wrap blocking work in `spawn_blocking`, copy out of a `std::sync` guard before `.await`, bound every channel, and keep every task in a `JoinHandle` or `JoinSet`.
- A pull request that adds a dependency says why.

## Tests

- Write one test per behaviour, through the module's interface, named as a sentence about that behaviour. Before adding one, check whether an existing test already goes red when the behaviour breaks.
- Leave to the compiler what it checks: field access, plain derives, constants.
- Reach for a fake or an in-memory duplex first. Spawn `keek` only for what takes a real process: the lock, the socket, inherited environment, exit codes.

## CI

- `.github/workflows/ci.yml` runs the gates in one job. A pull request that adds a job, a matrix or a paid runner states its cost in minutes.
- `.githooks/pre-push` runs the gates before a push. Enable it once per clone: `git config core.hooksPath .githooks`.
- CI runs on every pull request and every push to `main`, docs-only ones too. The `gates` job is a required check on `main`, so a path filter would leave a pull request unable to merge.
- A third-party action is pinned to a commit, with its version in a comment.
- Only `main` saves the build cache, and pull requests restore from it.
