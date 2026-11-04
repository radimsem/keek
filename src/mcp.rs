use crate::VERSION;
use crate::daemon_client::{self, DaemonClient, ProcessStarter};
use crate::ipc::{Event, Request, Response};
use crate::paths::Paths;
use crate::verifier_protocol::VERIFIER_PROTOCOL;
use anyhow::{Context, Result};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{ErrorData, ServerHandler, ServiceExt};
use serde_json::{Value, json};
use std::time::Duration;
const DAEMON_START_TIMEOUT: Duration = Duration::from_secs(2);
const HELP_REASON_MAX_CHARS: usize = 40;
const NOTIFY_GRACE: Duration = Duration::from_millis(250);
pub async fn serve() -> Result<()> {
    let paths = Paths::from_env()?;
    let executable = std::env::current_exe().context("could not find the keek executable")?;
    let daemon = daemon_client::connect_or_start(
        &paths,
        &mut ProcessStarter::new(executable),
        DAEMON_START_TIMEOUT,
    )
    .await?;
    KeekServer { daemon }
        .serve(rmcp::transport::stdio())
        .await
        .context("could not start the MCP server")?
        .waiting()
        .await
        .context("the MCP server stopped unexpectedly")?;
    Ok(())
}
struct KeekServer {
    daemon: DaemonClient,
}
fn own_tools() -> Vec<Tool> {
    [
        ("keek_status","Report the keek daemon's version, pid and open Agent Session count.",json!({"type":"object","properties":{}})),
        ("open_session","Open an Agent Session for a Verifier once its desktop is ready. Returns credentials as Test Credential names; use ${NAME} in tool arguments.",json!({"type":"object","properties":{"project":{"type":"string"},"worktree":{"type":"string"},"goal":{"type":"string"},"checks":{"type":"array","items":{"type":"string"}}},"required":["project","worktree","goal","checks"]})),
        ("add_check","Add a Check found while verifying.",json!({"type":"object","properties":{"session":{"type":"string"},"text":{"type":"string"}},"required":["session","text"]})),
        ("mark_check","Mark a Check passed, failed or skipped. Failed and skipped need a one-line comment.",json!({"type":"object","properties":{"session":{"type":"string"},"check":{"type":"integer"},"outcome":{"type":"string","enum":["passed","failed","skipped"]},"comment":{"type":"string"}},"required":["session","check","outcome"]})),
        ("request_help","Raise a Help Request with a reason shown to the developer. This call can block until the developer finishes a Takeover. The result starts with a note and a fresh screenshot to read before acting.",json!({"type":"object","properties":{"session":{"type":"string"},"reason":{"type":"string","minLength":1,"maxLength":HELP_REASON_MAX_CHARS}},"required":["session","reason"]})),
        ("close_session","Derive the Verdict and file Evidence. Returns incomplete in place of the Verdict when keek cannot stop the recording or the session opened a window on the developer's desktop. Refuses with no Checks or unmarked Checks. Returns video_error if the Verification Video could not be produced; the rest of the Evidence is still filed. Returns actions_error if Test Credentials could not be removed; the Action Log is withheld and kept in the session folder.",json!({"type":"object","properties":{"session":{"type":"string"},"summary":{"type":"string"}},"required":["session","summary"]}))
    ].into_iter().map(|(name,description,schema)|Tool::new(name,description,schema.as_object().cloned().unwrap_or_default())).collect()
}
fn can_wait(request: &Request) -> bool {
    match request {
        Request::OpenSession { .. }
        | Request::RequestHelp { .. }
        | Request::CallTool { .. }
        | Request::CloseSession { .. } => true,
        Request::Status
        | Request::ListTools
        | Request::AddCheck { .. }
        | Request::MarkCheck { .. }
        | Request::WatchSession { .. }
        | Request::Shutdown => false,
    }
}
fn tool_error(message: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message.into())])
}
impl ServerHandler for KeekServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("keek", VERSION))
            .with_instructions(VERIFIER_PROTOCOL)
    }
    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let mut tools = own_tools();
        if let Ok(Response::Tools(value)) = self.daemon.request(Request::ListTools).await
            && let Some(list) = value.get("tools").and_then(Value::as_array)
        {
            for tool in list {
                if let Ok(tool) = serde_json::from_value(tool.clone()) {
                    tools.push(tool);
                }
            }
        }
        Ok(ListToolsResult::with_all_items(tools))
    }
    async fn call_tool(
        &self,
        params: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let args = Value::Object(params.arguments.unwrap_or_default());
        let request=match params.name.as_ref(){
            "keek_status"=>Ok(Request::Status),
            "open_session"=>parse_open(args),
            "close_session"=>parse_close(args),
            "request_help"=>parse_help(args),
            "add_check"=>parse_add(args),
            "mark_check"=>parse_mark(args),
            tool=>args.get("session").and_then(Value::as_str).ok_or_else(||"Every forwarded tool requires an Agent Session name in session. Call open_session first.".to_owned()).and_then(|name|name.parse().map_err(|e:crate::session_folder::InvalidSessionName|e.to_string())).map(|session|Request::CallTool{session,tool:tool.into(),arguments:args}),
        };
        let request = match request {
            Ok(request) => request,
            Err(message) => return Ok(tool_error(message).into()),
        };
        let can_wait = can_wait(&request);
        let withdrawable = matches!(request, Request::RequestHelp { .. });
        let cancelled = async {
            if withdrawable {
                context.ct.cancelled().await;
            } else {
                std::future::pending::<()>().await;
            }
        };
        let (events, event_receiver) = tokio::sync::mpsc::channel(16);
        let response = match waiting_with_progress(
            self.daemon.request_until(request, Some(events), cancelled),
            event_receiver,
            if can_wait {
                context.meta.get_progress_token()
            } else {
                None
            },
            |notification| {
                let peer = context.peer.clone();
                async move {
                    let _ = peer.notify_progress(notification).await;
                }
            },
        )
        .await
        {
            Ok(response) => response,
            Err(error) => return Ok(tool_error(error.to_string()).into()),
        };
        Ok(match response {
            Response::ToolResult(value) => {
                serde_json::from_value(value).unwrap_or_else(|e| tool_error(e.to_string()))
            }
            Response::Refused { message } => tool_error(message),
            Response::Opened {
                credentials,
                session,
                checks,
                warnings,
            } => CallToolResult::structured(
                json!({"session":session,"checks":checks,"warnings":warnings,"credentials":credentials}),
            ),
            Response::CheckAdded { check } | Response::CheckMarked { check } => {
                CallToolResult::structured(json!({"check":check}))
            }
            Response::Closed(value) => CallToolResult::structured(value),
            Response::Status(status) => CallToolResult::structured(
                serde_json::to_value(status)
                    .map_err(|e| ErrorData::internal_error(e.to_string(), None))?,
            ),
            Response::Tools(_)
            | Response::WatchEnded
            | Response::ShuttingDown
            | Response::ShutdownRefused { .. } => {
                tool_error("Unexpected daemon response. Restart the keek MCP server.")
            }
        }
        .into())
    }
}
#[derive(serde::Deserialize)]
struct OpenArguments {
    project: String,
    worktree: std::path::PathBuf,
    goal: String,
    checks: Vec<String>,
}
fn parse_open(value: Value) -> Result<Request, String> {
    let args: OpenArguments = serde_json::from_value(value).map_err(|e| {
        format!("Cannot open an Agent Session: {e}. Pass project, worktree, goal and checks.")
    })?;
    if !args.worktree.is_absolute() {
        return Err("Cannot open an Agent Session: worktree must be an absolute path. Pass an absolute worktree path.".into());
    }
    Ok(Request::OpenSession {
        project: args.project,
        worktree: args.worktree,
        goal: args.goal,
        checks: args.checks,
    })
}
#[derive(serde::Deserialize)]
struct CloseArguments {
    session: crate::session_folder::SessionName,
    summary: String,
}
fn parse_close(value: Value) -> Result<Request, String> {
    let args: CloseArguments = serde_json::from_value(value)
        .map_err(|e| format!("Cannot close an Agent Session: {e}. Pass session and summary."))?;
    Ok(Request::CloseSession {
        session: args.session,
        summary: args.summary,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_session_refuses_a_relative_worktree() {
        let error =
            parse_open(json!({"project":"app","worktree":"relative","goal":"test","checks":[]}))
                .unwrap_err();
        assert!(error.contains("absolute path"), "{error}");
    }
}

#[derive(serde::Deserialize)]
struct AddArguments {
    session: crate::session_folder::SessionName,
    text: String,
}
fn parse_add(value: Value) -> Result<Request, String> {
    let args: AddArguments = serde_json::from_value(value)
        .map_err(|e| format!("Cannot add a Check: {e}. Call add_check with session and text."))?;
    Ok(Request::AddCheck {
        session: args.session,
        text: args.text,
    })
}
#[derive(serde::Deserialize)]
struct MarkArguments {
    session: crate::session_folder::SessionName,
    check: crate::timeline::CheckId,
    outcome: crate::timeline::CheckOutcome,
    #[serde(default)]
    comment: String,
}
fn parse_mark(value: Value) -> Result<Request, String> {
    let args: MarkArguments = serde_json::from_value(value).map_err(|e| format!("Cannot mark a Check: {e}. Call mark_check with session, check and outcome, plus a one-line comment when failed or skipped."))?;
    Ok(Request::MarkCheck {
        session: args.session,
        check: args.check,
        outcome: args.outcome,
        comment: args.comment,
    })
}

#[cfg(test)]
mod check_tests {
    use super::*;
    #[test]
    fn check_tools_parse_stable_ids_and_outcomes() {
        assert!(
            matches!(parse_add(json!({"session":"app_12-00-00","text":"criterion"})), Ok(Request::AddCheck { text, .. }) if text == "criterion")
        );
        assert!(
            matches!(parse_mark(json!({"session":"app_12-00-00","check":2,"outcome":"passed"})), Ok(Request::MarkCheck { check:crate::timeline::CheckId(2), outcome:crate::timeline::CheckOutcome::Passed, comment, .. }) if comment.is_empty())
        );
        for outcome in ["fail", "pass", "unknown"] {
            assert!(
                parse_mark(json!({"session":"app_12-00-00","check":2,"outcome":outcome})).is_err()
            );
        }
        let tools = own_tools();
        for name in ["add_check", "mark_check"] {
            assert!(tools.iter().any(|tool| tool.name == name));
        }
    }
}

#[cfg(test)]
#[path = "mcp_progress_tests.rs"]
mod progress_tests;

#[cfg(test)]
#[path = "mcp_cancel_tests.rs"]
mod cancel_tests;

/// Sends a progress notification for each `Event::Progress` and one every minute, until `request` ends.
async fn waiting_with_progress<F, T, N, NF>(
    request: F,
    mut events: tokio::sync::mpsc::Receiver<Event>,
    token: Option<rmcp::model::ProgressToken>,
    mut notify: N,
) -> T
where
    F: std::future::Future<Output = T>,
    N: FnMut(rmcp::model::ProgressNotificationParam) -> NF,
    NF: std::future::Future<Output = ()>,
{
    let Some(token) = token else {
        return request.await;
    };
    tokio::pin!(request);
    let mut progress = 0.0;
    let mut interval = tokio::time::interval_at(
        tokio::time::Instant::now() + Duration::from_secs(60),
        Duration::from_secs(60),
    );
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            biased;
            Some(event) = events.recv() => match event {
                Event::Progress { message } => {
                    progress += 1.0;
                    let notification = rmcp::model::ProgressNotificationParam::new(token.clone(), progress).with_message(message);
                    // A client that stops reading must not hold the reply, so a slow delivery is dropped.
                    let _ = tokio::time::timeout(NOTIFY_GRACE, notify(notification)).await;
                }
                Event::Snapshot(_) => {}
            },
            reply = &mut request => return reply,
            _ = interval.tick() => {
                progress += 1.0;
                let notification = rmcp::model::ProgressNotificationParam::new(token.clone(), progress);
                tokio::select! {
                    biased;
                    reply = &mut request => return reply,
                    () = notify(notification) => {}
                }
            }
        }
    }
}
#[derive(serde::Deserialize)]
struct HelpArguments {
    session: crate::session_folder::SessionName,
    reason: String,
}
fn parse_help(value: Value) -> Result<Request, String> {
    let args: HelpArguments = serde_json::from_value(value)
        .map_err(|e| format!("Cannot raise a Help Request: {e}. Pass session and reason."))?;
    if args.reason.trim().is_empty() {
        return Err(
            "Cannot raise a Help Request without a reason. Pass a non-empty reason.".into(),
        );
    }
    if args.reason.chars().count() > HELP_REASON_MAX_CHARS || args.reason.contains(['\n', '\r']) {
        return Err(format!(
            "The reason is shown on the Session Card, in a notification and in the Verification Video, so it must be one line of at most {HELP_REASON_MAX_CHARS} characters, such as \"2FA code\". Shorten it and call request_help again."
        ));
    }
    Ok(Request::RequestHelp {
        session: args.session,
        reason: args.reason,
    })
}
