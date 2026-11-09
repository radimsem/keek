# Test Files live beside the Project Profile

The trial's Verifier could not reach a git-ignored upload, and the untracked-folder workaround left client data one `git add` from a commit. Test Files therefore live outside the repository in `~/.config/keek/projects/<project>.files/`, beside the Project Profile, and each Agent Session sees them read-only at `/keek/files`. The developer places them there by hand.

## Considered Options

- A Project Profile key that lets listed git-ignored paths into the session tree. An ignored file does not exist in a fresh worktree, and the key opens a hole in the rule that hides `.env*`.
- `set_project_profile` copying host paths, as it does for Test Credentials. Any coding agent could then put any host file in front of the recorder without the developer seeing it.
- Documentation only. The untracked-folder workaround keeps client data one `git add` from a commit.

## Consequences

- Test Files are not part of the Project Layer key, so adding or changing one does not rebuild the Project Layer.
- The install sandbox has no Test Files bind. The Agent Session and the `prepare_project` probe bind them read-only at `/keek/files`.
- `open_session` lists at most 100 Test File names and counts the rest. The Verdict Report lists them in its Project Profile section.
- No keek tool writes to the Test Files folder. The developer places files there by hand, and names must be safe to appear in Evidence.
