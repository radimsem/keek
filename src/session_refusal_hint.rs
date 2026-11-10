use super::*;
use serde_json::json;

enum Refusal {
    WindowIds,
    StaleCapture,
}

fn classify(reply: &Value) -> Option<Refusal> {
    if reply["isError"] != true {
        return None;
    }
    let text = reply["content"]
        .as_array()?
        .iter()
        .filter(|item| item["type"] == "text")
        .filter_map(|item| item["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    if text.starts_with("Missing required integer field: pid")
        || text.starts_with("No windows found for pid ")
    {
        Some(Refusal::WindowIds)
    } else if text.contains("capture-bound click refused: capture id is unknown") {
        Some(Refusal::StaleCapture)
    } else {
        None
    }
}

pub(super) async fn add(driver: &mut dyn Driver, mut reply: Value) -> Value {
    let Some(refusal) = classify(&reply) else {
        return reply;
    };
    let hint = match refusal {
        Refusal::StaleCapture => "keek: that capture_id is unknown. Call get_window_state again and use the capture_id it returns.".to_owned(),
        Refusal::WindowIds => {
            let Some(text) = window_list(driver).await else {
                return reply;
            };
            format!("keek: Cua Driver refused this call because the pid or window_id is missing or matches no window. Windows in this Agent Session:\n{text}\nCall again with the pid and window_id of the window you mean. A dialog can belong to another process, so use the pid listed for its window.")
        }
    };
    if let Some(content) = reply["content"].as_array_mut() {
        content.push(json!({"type":"text", "text":hint}));
    }
    reply
}

async fn window_list(driver: &mut dyn Driver) -> Option<String> {
    let windows = call_driver(driver, "list_windows", json!({}), Duration::from_secs(10))
        .await
        .ok()?;
    if windows["isError"] == true {
        return None;
    }
    windows["content"]
        .as_array()?
        .iter()
        .find(|item| item["type"] == "text" && item["text"].is_string())?["text"]
        .as_str()
        .map(str::to_owned)
}
