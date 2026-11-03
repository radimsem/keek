use super::*;
use crate::checklist::IncompleteReason;
impl Actor {
    pub(super) async fn hand_back(
        &self,
        driver: &mut dyn Driver,
        span: gate::Span,
        opener: &mut Option<watch::Receiver<()>>,
        deadline: &mut tokio::time::Instant,
    ) -> Result<Response, IncompleteReason> {
        let mut ended = driver.ended();
        let capture = self.capture_hand_back(driver, span);
        tokio::pin!(capture);
        tokio::select! { biased;
                () = &mut ended => Err(IncompleteReason::DesktopStopped),
                response = &mut capture => {
                    *deadline = tokio::time::Instant::now() + self.idle_limit;
                    Ok(response)
                },
                () = self.opener_gone(opener) => Err(IncompleteReason::OpenerLeft),
        }
    }
    async fn capture_hand_back(&self, driver: &mut dyn Driver, mut span: gate::Span) -> Response {
        let _permit = loop {
            match self.control.gate.enter().await {
                gate::Entry::Run(permit) => break permit,
                gate::Entry::HandBack(current) => span = current,
            }
        };
        let seconds = span.end.saturating_duration_since(span.start).as_secs();
        let length = format!("{}:{:02}", seconds / 60, seconds % 60);
        let reason = span
            .reason
            .map(|reason| format!(" ({reason})"))
            .unwrap_or_default();
        let note = format!(
            "The developer controlled the session for {length}{reason}. The screen may have changed."
        );
        let capture = call_driver(
            driver,
            "get_desktop_state",
            json!({}),
            Duration::from_secs(150),
        )
        .await
        .and_then(|result| {
            if result.get("isError").and_then(Value::as_bool) == Some(true) {
                Err(DriverError::Call {
                    tool: "get_desktop_state".into(),
                    reason: result.to_string(),
                })
            } else {
                Ok(result)
            }
        });
        match capture {
            Ok(mut result) => {
                let mut content = vec![json!({"type":"text", "text":note})];
                if let Some(items) = result.get_mut("content").and_then(Value::as_array_mut) {
                    content.append(items);
                }
                if let Some(object) = result.as_object_mut() {
                    object.insert("content".into(), json!(content));
                } else {
                    result = json!({"content":content});
                }
                Response::ToolResult(result)
            }
            Err(error) => {
                tracing::warn!(%error, "Could not take a fresh screenshot after Takeover");
                Response::ToolResult(
                    json!({"content":[{"type":"text", "text":note}, {"type":"text", "text":"keek couldn't take a fresh screenshot. Take one before you act."}]}),
                )
            }
        }
    }
}
