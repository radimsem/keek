# A fresh Verifier checks each change, and its Verdict is final

An agent that grades its own change is inclined to pass it, and a Verdict Report is only worth attaching to a PR if it was not written by the author. So the coding agent never drives an Agent Session itself. For each verification it starts a fresh subagent, the Verifier, and gives it only the goal written as observable acceptance criteria, the Project and the worktree. The Verifier judges what it sees on screen, not what the diff intends. Its Verdict is final: nobody edits a Verdict Report, and a different outcome needs a new Agent Session. After Needs fixes, the coding agent may fix the change and start a new Verifier, up to 3 Agent Sessions per change, then stops and reports to the developer.

## Considered Options

- The coding agent verifies its own change: the most context and the cheapest, but it grades its own work.
- keek launches its own agent harness per session, as `cua agent run` does: independent of the coding agent, but keek would need model credentials and a harness of its own.
- Give the Verifier the diff or the coding agent's notes as well: it could aim at changed paths, but it would read the intent and expect it.

## Consequences

- keek can't tell a Verifier from the coding agent, because MCP calls carry no caller identity. The separation and the retry cap hold by instruction only ([ADR 0011](0011-mcp-server-only.md)).
- keek's MCP server belongs to the coding agent's session, so a Verifier that ends without `close_session` ends no process keek can watch. Five minutes without a tool call, outside a Help Request or Takeover, ends the session as an Incomplete Session.
- Every attempt keeps its Evidence and its line in the Project's `index.md`.
