# keek ships as an MCP server only

keek must work with whichever coding agent the developer uses, not only Claude Code. It ships as one MCP server, with no Claude Code plugin, subagent definition or skill. The server's instructions describe the Verifier protocol: write the goal as acceptance criteria, start a fresh subagent with only the goal, the Project and the worktree, and after Needs fixes start a new Verifier, up to 3 Agent Sessions per change. Any harness that can start a subagent can follow it.

## Considered Options

- A Claude Code plugin with the MCP server, a `keek-verifier` subagent definition and a skill for the coding agent. Claude Code plugins can bundle all three, but the result would serve Claude Code only.
- The plugin plus the same protocol in the server's instructions for other harnesses: two copies of the protocol to keep in step.

## Consequences

- keek can't choose the Verifier's model or limit its tools; the coding agent's harness does.
- `request_help` blocks until the Takeover ends and sends progress notifications while it waits. Claude Code aborts a local MCP call after 30 minutes with no response or progress.

## Amendment: one setup skill

Amended on 2026-11-04 by [ADR 0016](0016-a-project-profile-not-a-setup-script-or-dockerfile.md). keek ships the MCP server plus one skill, `skills/keek-setup/SKILL.md`, which tells the coding agent how to read a repository and write its Project Profile. A skill is no longer specific to Claude Code: Codex 0.166 loads `SKILL.md` from `~/.agents/skills`, as Claude Code does from `~/.claude/skills`, and the developer copies it into both by hand. The Verifier protocol stays in the server's instructions. Codex's `workspace-write` sandbox can't write `~/.config/keek`, so the skill has the agent call `set_project_profile` and keek writes the profile itself.
