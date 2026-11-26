use super::*;
use rmcp::model::Tool;
use serde_json::json;

const DELEGATE_STEP_DESCRIPTION: &str = "Hand one routine step to keek's Decision Model, for a target that is clear from its words, such as a named button, field or link. keek reads the window once, asks the model which element the intent means, and acts only when the model is confident. Otherwise it acts on nothing and returns the top candidates, and you act yourself. It never marks a Check.";

impl Registry {
    pub(super) fn list_tools(&mut self, reply: oneshot::Sender<Response>) {
        let mut catalog = self.catalog.clone();
        let delegate_step = self.dependencies.delegate.is_some();
        self.background.spawn(async move {
            let result = wait_catalog(&mut catalog).await;
            let mut tools = result.map_or_else(|_| vec![], |c| c.tools());
            if delegate_step {
                tools.push(delegate_step_tool());
            }
            let _ = reply.send(Response::Tools(serde_json::json!({"tools":tools})));
        });
    }

    pub(super) async fn status(&self) -> crate::ipc::DaemonStatus {
        crate::ipc::DaemonStatus {
            version: crate::VERSION.into(),
            build: crate::build_id::current()
                .await
                .map(|build| build.to_string()),
            pid: std::process::id(),
            open_sessions: self.len(),
            decision_model: self.dependencies.delegate.as_ref().map(|delegate| {
                crate::ipc::DecisionModelStatus {
                    endpoint: delegate.config.endpoint.clone(),
                    model: delegate.config.model.clone(),
                    threshold: delegate.config.threshold,
                }
            }),
        }
    }
}

fn delegate_step_tool() -> Tool {
    let schema = json!({
        "type": "object",
        "properties": {
            "session": {"type": "string", "description": "The name of the open keek Agent Session returned by open_session."},
            "intent": {"type": "string", "description": "The step in the words of the Check, such as \"click Sign in\"."},
            "action": {"type": "string", "enum": ["click", "double_click", "right_click", "type", "select"]},
            "text": {"type": "string", "description": "The text to type. Required for type."},
            "option": {"type": "string", "description": "The option to pick. Required for select."},
            "window": {
                "type": "object",
                "description": "The window to read and act in, as for get_window_state.",
                "properties": {"pid": {"type": "integer"}, "window_id": {"type": "integer"}},
                "required": ["pid"]
            }
        },
        "required": ["session", "intent", "action", "window"]
    });
    Tool::new(
        crate::session::delegate::TOOL,
        DELEGATE_STEP_DESCRIPTION,
        schema.as_object().cloned().unwrap_or_default(),
    )
}
