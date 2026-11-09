use super::*;

/// Cua Driver stops its recording after 300 s without a tool call
/// (`CUA_DRIVER_RS_RECORDING_IDLE_TTL_SECS`), and keek holds the Verifier's calls through a Help
/// Request and a Takeover. A read-only call each minute keeps the recording running.
const RECORDING_KEEPALIVE: Duration = Duration::from_secs(60);

pub(super) fn timer() -> tokio::time::Interval {
    let mut timer = tokio::time::interval_at(
        tokio::time::Instant::now() + RECORDING_KEEPALIVE,
        RECORDING_KEEPALIVE,
    );
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    timer
}

pub(super) async fn keep_recording(driver: &mut dyn Driver) {
    if let Err(error) = call_driver(
        driver,
        "get_recording_state",
        json!({}),
        Duration::from_secs(10),
    )
    .await
    {
        tracing::warn!(%error, "could not keep the recording alive while the Verifier waits");
    }
}
