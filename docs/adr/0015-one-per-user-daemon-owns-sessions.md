# One per-user daemon owns every Agent Session

Each coding agent starts its own `keek mcp`, but the session cap, the slots on the Agents Workspace and the Session Footers are shared across all of them. So `keek mcp` is a thin client of one per-user `keek daemon`, which it starts when none is running. The daemon owns every open Agent Session, the connection to each Cua Driver, the Session Timeline and the Session Footers, and exits once no client, session or Session Card remains. This doesn't change what keek ships: the coding agent still sees one MCP server ([ADR 0011](0011-mcp-server-only.md)).

## Considered Options

- No daemon: each `keek mcp` owns its sessions, shares the cap and slots through lock files, and spawns a footer child per session. No lifecycle or version-skew work, but shared state is spread across files and processes.

## Consequences

- A session ends as an Incomplete Session when the `keek mcp` that opened it disconnects, so a coding agent's exit still ends its sessions.
- If the daemon dies, its sandboxes die with it. The next daemon finishes any session it finds unfiled as an Incomplete Session.
- `keek mcp` and the daemon can differ in build after an update. Builds are ordered by semver version, then by the running executable's modification time; a missing build is older than any known build of the same version. If the client cannot read its own build, it compares versions only and logs one warning. With no session open, an older daemon exits and a new one starts. With a session open, `keek mcp` refuses and names both versions and, when equal, both build timestamps in UTC. An older `keek mcp` also refuses a newer daemon, so two installed builds can't keep evicting each other's daemon.
- The daemon needs the host's Wayland and Hyprland environment, and takes it from the `keek mcp` that starts it.
