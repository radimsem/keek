# The agent names the Project when it opens an Agent Session

Evidence from every worktree of an app is filed under one Project, in `~/Videos/keek/<project>/<date>/`, so a Project can't be a worktree. keek could derive the name from git or require a committed one. Instead the coding agent names it, the Verifier passes it to `open_session`, and keek accepts any name. One repository can therefore hold several Projects, such as a monorepo's web and desktop apps. The Project Profile is per Project, so each Project names its own install and start commands, which also receive the name as `KEEK_PROJECT`. A misspelled name files Evidence under a new Project folder and finds no Test Credentials. `open_session` refuses a Project that has no Project Profile and warns about one that has no Test Credentials, so the agent can notice.

## Considered Options

- Derive the name from the main worktree's folder, with an override committed in `.keek/`.
- Bind the first name used from a repository and refuse any other.
- Accept only names the developer registered with `keek project add`.

## Consequences

- The Project Profile and Test Credentials live in `~/.config/keek/projects/<name>.toml` and `<name>.env`. A coding agent writes the profile through `set_project_profile` ([ADR 0016](0016-a-project-profile-not-a-setup-script-or-dockerfile.md)), and the developer fills in Test Credentials keek could not copy.
- Evidence lives outside the repository, so it survives removing a worktree or a clone.
