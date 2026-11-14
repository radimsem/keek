# Releasing

A release is a `v<version>` tag on `main` and a GitHub release whose notes are that version's section of `CHANGELOG.md`. Tagging and publishing are the developer's to do or approve.

## Between releases

A pull request that changes what a person using keek would notice adds a line under `Unreleased` in `CHANGELOG.md`. `AGENTS.md` has the rule.

## Cutting a release

1. Branch as `chore/release-<version>` and change three files:
   - `version` in `Cargo.toml`, then `cargo check` to carry it into `Cargo.lock`.
   - The `--tag` in the README's install command and the tag in its raw skill URL. `the_readme_installs_the_crate_version` fails until both match.
   - `CHANGELOG.md`: rename `Unreleased` to `## <version> - <date>` and add an empty `## Unreleased` above it. Links in the notes are absolute, because a GitHub release doesn't resolve relative ones.
2. Read the notes as GitHub will show them.

   ```sh
   scripts/release-notes.sh <version>
   ```

3. Open the pull request. The developer approves the exact text of the notes there.
4. After the merge, fetch `main`, find the commit and check that its CI run is green.

   ```sh
   git fetch origin main
   gh run list --branch main --workflow CI --limit 3
   ```

5. Tag that commit and push the tag. The Release workflow checks that the tag matches `Cargo.toml` and sits on `main`, then drafts the GitHub release from the changelog. It takes about a minute.

   ```sh
   git tag v<version> <commit>
   git push origin v<version>
   ```

6. Install from the tag with a clean cargo home and check the version.

   ```sh
   root="$(mktemp -d)"
   CARGO_HOME="$(mktemp -d)" cargo install --git https://github.com/radimsem/keek --tag v<version> --locked --root "$root"
   "$root/bin/keek" --version
   ```

   Then open one Agent Session with that binary and take it to a Verdict. A coding agent starts the `keek` on its `PATH`, so register `$root/bin/keek mcp` for this run.

7. The developer publishes the draft.

   ```sh
   gh release edit v<version> --draft=false
   ```

## When a tag is wrong

The `Protect release tags` ruleset refuses to delete or move a `v*` tag. Before the release is published, disable that ruleset under Settings, Rules, delete the draft and the tag, and enable the ruleset again. Then fix `main` and tag again.

A published release is immutable. Leave its tag alone and release a new patch version.
