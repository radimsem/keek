/// keek's MCP server instructions, which are the only way the Verifier protocol
/// reaches a coding agent (ADR 0011). Keep them in step with the Verifier protocol
/// section of `docs/design.md`.
pub const VERIFIER_PROTOCOL: &str = r#"keek verifies a change on a sandboxed desktop. Never drive an Agent Session yourself: a fresh Verifier does, so a Verdict never comes from the agent that wrote the change.

To verify a change:

1. Write the goal as a short list of acceptance checks a person could verify on screen ("Signing in with the test account lands on the dashboard"). They become the Verification Checklist.
2. Start a fresh subagent, the Verifier, with only the goal and its checks, the Project name and the worktree. No diff and no notes on what changed.
3. Read the Verifier's Verdict and Evidence paths. A Verdict is final.
4. After Needs fixes, read the failed Checks and their comments, fix the change and start a new Verifier, up to 3 Agent Sessions per change. Then stop and report to the developer with every session's Evidence. After Partly verified, tell the developer which Checks were skipped and why.

To wait for the screen to change, call `get_window_state` again rather than sleeping in a shell.

When a decision model is configured, use `delegate_step` for a routine step whose target is clear from its words, such as a named button, field or link. Look at the screen yourself before you mark a Check. After 'No action taken', act yourself.

`open_session` returns `credentials`, the names of the Project's Test Credentials. To use one, write `${NAME}` in a tool argument, including inside `run_steps`. keek substitutes the value before forwarding and masks secret values in results, so the Verifier never sees a secret value. Public values, such as a test username, stay readable in results and Evidence. An unknown name is refused. In `run_script`, substitution is plain text inside JavaScript, so prefer `run_steps` for a value with quotes or backslashes.

`open_session` also returns `files`, the names of the Project's Test Files: sample inputs, such as a document to upload, that the developer placed outside the repository. They are read-only at `/keek/files/<name>`; in a file dialog press Ctrl+L and type that path. `more_files` counts names left off a long list. Git-ignored files are absent from `/keek/worktree`, so a path to one fails: use a Test File, or mark the Check skipped and say why.

Call `mark_check` for each Check as soon as you observe it, rather than marking all Checks at the end, because its mark time is where the Verdict Report and Verification Video point.

Judge each Check only from what the Agent Session shows. Don't read the worktree's source or run host commands to explain a result.

`type_text` into a password field returns `effect: unverifiable`, and the field can't be read back. Click the field and clear it with ctrl+a before typing, so a retype doesn't double the password."#;

#[cfg(test)]
mod tests {
    use super::VERIFIER_PROTOCOL;

    #[test]
    fn the_verifier_protocol_mentions_delegate_step() {
        for required in [
            "delegate_step",
            "Look at the screen yourself before you mark a Check",
            "After 'No action taken', act yourself",
        ] {
            assert!(VERIFIER_PROTOCOL.contains(required), "lacks {required:?}");
        }
    }
}
