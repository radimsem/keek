use crate::session_folder::SessionName;
use rmcp::model::Tool;
use serde_json::{Value, json};

const DENIED: &[&str] = &[
    "start_recording",
    "stop_recording",
    "get_recording_state",
    "replay_trajectory",
    "start_session",
    "end_session",
    "escalate_session",
    "set_config",
    "check_for_update",
    "install_extension",
    "install_ffmpeg",
    "set_agent_cursor_enabled",
    "set_agent_cursor_motion",
    "set_agent_cursor_theme",
];

#[derive(Debug, Clone, Default)]
pub struct Catalog {
    tools: Vec<Tool>,
}
impl Catalog {
    pub fn new(tools: Vec<Tool>) -> Self {
        Self {
            tools: tools
                .into_iter()
                .filter(|tool| !DENIED.contains(&tool.name.as_ref()))
                .collect(),
        }
    }
    pub fn tools(&self) -> Vec<Tool> {
        self.tools.iter().map(|tool| {
            let mut exposed = tool.clone();
            let schema = std::sync::Arc::make_mut(&mut exposed.input_schema);
            let properties = schema.entry("properties").or_insert_with(|| json!({}));
            if let Some(properties) = properties.as_object_mut() {
                properties.insert("session".into(), json!({"type":"string", "description":"The name of the open keek Agent Session returned by open_session."}));
                if let Some(delivery) = properties.get_mut("delivery_mode").and_then(Value::as_object_mut) {
                    delivery.insert("default".into(), json!("foreground"));
                }
            }
            let mut required = schema.get("required").and_then(Value::as_array).cloned().unwrap_or_default();
            required.retain(|value| value != "session" && value != "delivery_mode");
            required.push(json!("session"));
            schema.insert("required".into(), required.into());
            exposed
        }).collect()
    }
    pub fn prepare_call(
        &self,
        name: &str,
        session: &SessionName,
        arguments: Value,
    ) -> Result<(Value, bool), CatalogError> {
        let tool = self
            .tools
            .iter()
            .find(|tool| tool.name == name)
            .ok_or_else(|| CatalogError::UnknownTool(name.to_owned()))?;
        let mut arguments = arguments
            .as_object()
            .ok_or(CatalogError::Arguments)?
            .clone();
        arguments.remove("session");
        if tool
            .input_schema
            .get("required")
            .and_then(Value::as_array)
            .is_some_and(|required| required.iter().any(|value| value == "session"))
        {
            arguments.insert("session".into(), json!(session.to_string()));
        }
        if tool
            .input_schema
            .get("properties")
            .and_then(|properties| properties.get("delivery_mode"))
            .is_some()
        {
            arguments
                .entry("delivery_mode")
                .or_insert_with(|| json!("foreground"));
        }
        let action = !tool
            .annotations
            .as_ref()
            .is_some_and(|hints| hints.read_only_hint == Some(true));
        Ok((arguments.into(), action))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("{0}")]
    Unavailable(String),
    #[error(
        "Cua Driver tool {0:?} is not exposed by keek. List tools and choose an available tool."
    )]
    UnknownTool(String),
    #[error("Cua Driver arguments must be a JSON object. Pass the tool's named arguments.")]
    Arguments,
}

pub async fn load(
    runtime_dir: &std::path::Path,
    home: &std::path::Path,
) -> Result<Catalog, CatalogError> {
    use crate::driver::{CuaDriver, DRIVER_VERSION, Driver, driver_path};
    use std::process::Stdio;
    use std::time::Duration;

    let binary = driver_path(home);
    let unavailable = |reason: String| {
        CatalogError::Unavailable(format!(
            "Cua Driver catalog is unavailable at {}: {reason}. Install Cua Driver {DRIVER_VERSION} at this path and restart the keek daemon.",
            binary.display()
        ))
    };
    let scratch = runtime_dir.join(format!("catalog-home-{}", std::process::id()));
    let scratch_copy = scratch.clone();
    tokio::task::spawn_blocking(move || std::fs::create_dir(&scratch_copy))
        .await
        .map_err(|error| unavailable(error.to_string()))?
        .map_err(|error| unavailable(error.to_string()))?;
    let mut command = tokio::process::Command::new(&binary);
    command
        .arg("mcp")
        .env_clear()
        .env("HOME", &scratch)
        .env("CUA_DRIVER_RS_TELEMETRY_ENABLED", "false")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let result = async {
        let mut child = command
            .spawn()
            .map_err(|error| unavailable(error.to_string()))?;
        let exchange = async {
            let reader = child
                .stdout
                .take()
                .ok_or_else(|| unavailable("stdout was not piped".into()))?;
            let writer = child
                .stdin
                .take()
                .ok_or_else(|| unavailable("stdin was not piped".into()))?;
            let mut driver = CuaDriver::connect(reader, writer)
                .await
                .map_err(|error| unavailable(error.to_string()))?;
            let tools = driver
                .list_tools()
                .await
                .map_err(|error| unavailable(error.to_string()));
            let closed = driver
                .close()
                .await
                .map_err(|error| unavailable(error.to_string()));
            closed?;
            tools.map(Catalog::new)
        };
        let result = tokio::time::timeout(Duration::from_secs(30), exchange)
            .await
            .map_err(|_| unavailable("listing tools timed out after 30 seconds".into()))
            .and_then(|value| value);
        match tokio::time::timeout(Duration::from_secs(10), child.wait()).await {
            Ok(waited) => {
                waited.map_err(|error| unavailable(error.to_string()))?;
            }
            Err(_) => {
                child
                    .kill()
                    .await
                    .map_err(|error| unavailable(error.to_string()))?;
            }
        }
        result
    }
    .await;
    tokio::task::spawn_blocking(move || std::fs::remove_dir_all(scratch))
        .await
        .map_err(|error| unavailable(error.to_string()))?
        .map_err(|error| unavailable(error.to_string()))?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tool(name: &str, schema: Value) -> Tool {
        Tool::new(
            name.to_owned(),
            "fixture",
            schema.as_object().unwrap().clone(),
        )
    }

    #[tokio::test]
    async fn an_uninstalled_driver_names_the_pinned_path_and_installation() {
        let dir = tempfile::tempdir().unwrap();
        let error = load(dir.path(), dir.path()).await.unwrap_err().to_string();
        assert!(error.contains(crate::driver::driver_path(dir.path()).to_str().unwrap()));
        assert!(error.to_ascii_lowercase().contains("install"));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn forwarding_strips_optional_session_and_defaults_delivery_mode() {
        let catalog = Catalog::new(vec![tool(
            "click",
            json!({"type":"object","properties":{"session":{"type":"string"},"delivery_mode":{"type":"string"}}}),
        )]);
        let name: SessionName = "app_12-00-00".parse().unwrap();
        let (args, action) = catalog
            .prepare_call("click", &name, json!({"session":"app_12-00-00","x":1}))
            .unwrap();
        assert_eq!(args, json!({"x":1,"delivery_mode":"foreground"}));
        assert!(action);
    }

    #[test]
    fn forwarding_rewrites_required_session_and_keeps_explicit_delivery_mode() {
        let catalog = Catalog::new(vec![tool(
            "browser_pointer",
            json!({"type":"object","properties":{"session":{"type":"string"},"delivery_mode":{"type":"string"}},"required":["session"]}),
        )]);
        let name: SessionName = "app_12-00-00".parse().unwrap();
        let (args, _) = catalog
            .prepare_call(
                "browser_pointer",
                &name,
                json!({"session":"wrong","delivery_mode":"background"}),
            )
            .unwrap();
        assert_eq!(
            args,
            json!({"session":"app_12-00-00","delivery_mode":"background"})
        );
    }

    #[test]
    fn only_read_only_hint_true_marks_a_call_as_reading() {
        let read = tool("read", json!({"type":"object"}))
            .with_annotations(rmcp::model::ToolAnnotations::new().read_only(true));
        let catalog = Catalog::new(vec![read, tool("run_steps", json!({"type":"object"}))]);
        let name: SessionName = "app_12-00-00".parse().unwrap();
        assert!(!catalog.prepare_call("read", &name, json!({})).unwrap().1);
        assert!(
            catalog
                .prepare_call("run_steps", &name, json!({}))
                .unwrap()
                .1
        );
    }

    #[test]
    fn every_denied_tool_is_absent() {
        let denied = [
            "start_recording",
            "stop_recording",
            "get_recording_state",
            "replay_trajectory",
            "start_session",
            "end_session",
            "escalate_session",
            "set_config",
            "check_for_update",
            "install_extension",
            "install_ffmpeg",
            "set_agent_cursor_enabled",
            "set_agent_cursor_motion",
            "set_agent_cursor_theme",
        ];
        let mut tools: Vec<_> = denied
            .iter()
            .map(|name| tool(name, json!({"type":"object"})))
            .collect();
        tools.push(tool("click", json!({"type":"object"})));
        let catalog = Catalog::new(tools);
        assert_eq!(
            catalog
                .tools()
                .iter()
                .map(|t| t.name.as_ref())
                .collect::<Vec<_>>(),
            ["click"]
        );
    }

    #[test]
    fn every_forwarded_schema_requires_keeks_session() {
        let catalog = Catalog::new(vec![tool(
            "click",
            json!({"type":"object", "properties":{"session":{"type":"string","description":"driver"}}}),
        )]);
        let tools = catalog.tools();
        let schema = tools.first().unwrap().input_schema.as_ref();
        assert_eq!(schema.get("required"), Some(&json!(["session"])));
        assert!(
            schema["properties"]["session"]["description"]
                .as_str()
                .unwrap()
                .contains("Agent Session")
        );
    }

    #[test]
    fn delivery_mode_is_optional_and_defaults_to_foreground_in_schema() {
        let catalog = Catalog::new(vec![tool(
            "click",
            json!({"type":"object", "properties":{"delivery_mode":{"type":"string"}}, "required":["delivery_mode"]}),
        )]);
        let tools = catalog.tools();
        let schema = tools.first().unwrap().input_schema.as_ref();
        assert_eq!(
            schema["properties"]["delivery_mode"]["default"],
            "foreground"
        );
        assert_eq!(schema["required"], json!(["session"]));
    }
}
