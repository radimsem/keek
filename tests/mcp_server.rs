mod common;

use anyhow::{Context, Result};
use common::TestEnv;
use keek::verifier_protocol::VERIFIER_PROTOCOL;

#[tokio::test]
async fn an_agent_can_read_the_verifier_protocol_and_list_tools() -> Result<()> {
    let env = TestEnv::new()?;
    let client = env.connect_mcp().await?;

    let info = client.peer_info().context("the server sent no info")?;
    let instructions = info
        .instructions
        .as_deref()
        .context("the server sent no instructions")?;
    for required in [
        "3 Agent Sessions",
        "acceptance checks",
        "fresh subagent",
        "No diff",
        "A Verdict is final",
        "Needs fixes",
        "Partly verified",
    ] {
        assert!(
            instructions.contains(required),
            "instructions lack {required:?}: {instructions}"
        );
    }

    let tools = client.list_all_tools().await?;
    let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_ref()).collect();
    assert_eq!(names, ["keek_status"]);

    client.cancel().await?;
    Ok(())
}

#[test]
fn the_instructions_state_each_step_exactly_as_the_design_does() {
    let design = include_str!("../docs/design.md");
    let steps: Vec<&str> = VERIFIER_PROTOCOL
        .lines()
        .filter(|line| line.starts_with(|first: char| first.is_ascii_digit()))
        .collect();

    assert_eq!(steps.len(), 4);
    for step in steps {
        assert!(design.contains(step), "docs/design.md lacks: {step}");
    }
}

#[tokio::test]
async fn keek_status_reports_the_daemon_version_and_open_sessions() -> Result<()> {
    let env = TestEnv::new()?;
    let client = env.connect_mcp().await?;

    let status = common::keek_status(&client).await?;

    assert_eq!(status.version, env!("CARGO_PKG_VERSION"));
    assert_eq!(status.open_sessions, 0);
    client.cancel().await?;
    Ok(())
}
