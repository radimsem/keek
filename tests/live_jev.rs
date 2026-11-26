use anyhow::Result;
use keek::config::DecisionModelConfig;
use keek::decision_model::{DecisionModel, DecisionRequest, JevClient};
use keek::step_candidates::{StepAction, candidates};
use serde_json::Value;

#[tokio::test]
#[ignore = "sends the made-up fixture window state to the hosted Jev API and needs KEEK_JEV_KEY"]
async fn live_jev_picks_sign_in_from_the_fixture() -> Result<()> {
    if std::env::var("KEEK_JEV_KEY").is_err() {
        eprintln!("skipped: export KEEK_JEV_KEY to run this test");
        return Ok(());
    }
    let state: Value = serde_json::from_str(include_str!("fixtures/window_state_signin.json"))?;
    let intent = "click Sign in";
    let client = JevClient::new(&DecisionModelConfig {
        endpoint: "https://api.typesafe.ai".to_owned(),
        model: "jev-latest".to_owned(),
        api_key_env: Some("KEEK_JEV_KEY".to_owned()),
        threshold: 0.9,
        timeout_ms: 10_000,
    })?;

    let decision = client
        .decide(DecisionRequest {
            intent: intent.to_owned(),
            action: StepAction::Click,
            window_title: Some("Sign in".to_owned()),
            candidates: candidates(&state, StepAction::Click, intent)?,
        })
        .await?;

    assert_eq!(decision.choice, "c1");
    assert!(decision.probabilities["c1"] >= 0.9, "{decision:?}");
    assert!(decision.matches >= 0.9, "{decision:?}");
    Ok(())
}
