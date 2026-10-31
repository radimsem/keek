# The agent names the Project when it opens an Agent Session

Evidence from every worktree of an app is filed under one Project, in `~/Videos/keek/<project>/<date>/`, so a Project can't be a worktree. keek could derive the name from git or require a committed one. Instead the coding agent names it, the Verifier passes it to `open_session`, and keek accepts any name. One repository can therefore hold several Projects, such as a monorepo's web and desktop apps. The Setup Script receives the name and starts the matching app. A misspelled name files Evidence under a new Project folder and finds no Test Credentials. `open_session` reports a Project that has no Test Credentials or settings, so the agent can notice.

## Considered Options

- Derive the name from the main worktree's folder, with an override committed in `.keek/`.
- Bind the first name used from a repository and refuse any other.
- Accept only names the developer registered with `keek project add`.

## Consequences

- Per-Project settings (window size, host ports to forward) and Test Credentials live in `~/.config/keek/projects/<name>.toml` and `<name>.env`. Each developer sets them up by hand.
- Evidence lives outside the repository, so it survives removing a worktree or a clone.
