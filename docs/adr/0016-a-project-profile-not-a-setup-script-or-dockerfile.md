# A Project Profile, not a Setup Script or Dockerfile

keek has to know how to install a Project's dependencies and start its App Under Test, and the answer depends on the machine as much as on the repository: which toolchain versions exist, which host ports a database listens on, how large the Session Window is. A committed `.keek/setup.sh` put that machine setup in the repository, and a coding agent had to commit it before the first Agent Session. So each Project has a Project Profile in `~/.config/keek/projects/<project>.toml`, outside the repository: its `install` and `start` commands, pinned toolchains, non-secret `[env]` values, forwarded ports and window size. A coding agent writes it through the `set_project_profile` tool after reading the repository, and tries it with `prepare_project`. keek writes the file itself, because Codex's `workspace-write` sandbox can't write `~/.config/keek`. The developer rarely edits it by hand.

## Considered Options

- A committed Setup Script, as before. A git-ignored one doesn't exist in a fresh worktree, and a committed one puts one developer's machine setup into every clone of the repository.
- A Dockerfile. The App Under Test has to run in the same sandbox as the host's nested Hyprland, Chromium and Cua Driver, on host libraries, with no root, so it can't run in a container image. App services such as a database run in Docker on the host and reach the App Under Test through a forwarded port.
- Detection in keek, reading `.nvmrc`, lockfiles and `package.json` itself. The coding agent already reads the repository, so the same rules would live in two places, and keek's copy would lag behind every new tool.

## Consequences

- `.keek/setup.sh` is no longer read. A Project that had one needs a Project Profile before its next `open_session`, which refuses without one and names `set_project_profile`.
- The Project Layer key covers the install command, the toolchain pins, the `[env]` values and the tracked lockfiles, in place of the Setup Script's content.
- A commit no longer says what built the environment, so the Verdict Report lists the profile's install and start commands, its toolchain versions and its `[env]` names.
- A profile is per machine and per developer. Two developers on one repository each have their own, written by their own coding agent.
