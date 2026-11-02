use crate::{
    compositor::{Placement, WindowPid},
    hypr,
    process::ProcessError,
    project::WindowSize,
    slots,
    viewer::SlotId,
};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

const DEFAULT_OUTER_GAP: i32 = 24;
const DEFAULT_INNER_GAP: i32 = 5;
fn refused(message: impl Into<String>) -> ProcessError {
    ProcessError::Failed {
        program: "Session Window placement".into(),
        message: message.into(),
    }
}
async fn json(request: &str) -> Result<Value, ProcessError> {
    serde_json::from_str(&hypr::command(request).await?).map_err(|e| {
        refused(format!(
            "Cannot read Hyprland geometry: {e}. Check the host compositor"
        ))
    })
}
fn number(v: &Value, key: &str) -> i32 {
    v.get(key)
        .and_then(Value::as_i64)
        .and_then(|n| n.try_into().ok())
        .unwrap_or(0)
}
pub async fn slot(
    index: SlotId,
    count: usize,
    size: Option<WindowSize>,
) -> Result<Placement, ProcessError> {
    let monitors = json("j/monitors").await?;
    let monitors = monitors
        .as_array()
        .ok_or_else(|| refused("No host monitor. Connect a monitor and retry"))?;
    let workspaces = json("j/workspaces").await?;
    let workspace_monitor = workspaces
        .as_array()
        .and_then(|entries| entries.iter().find(|w| w["name"] == "keek"))
        .and_then(|w| w["monitor"].as_str());
    let monitor = workspace_monitor
        .and_then(|name| monitors.iter().find(|m| m["name"] == name))
        .or_else(|| {
            monitors
                .iter()
                .find(|m| m["activeWorkspace"]["name"] == "keek")
        })
        .or_else(|| monitors.iter().find(|m| m["focused"] == true))
        .or_else(|| monitors.first())
        .ok_or_else(|| refused("No host monitor. Connect a monitor and retry"))?;
    let scale = monitor["scale"].as_f64().unwrap_or(1.0).max(0.1);
    let dimension = |field: &str| {
        monitor
            .get(field)
            .and_then(Value::as_i64)
            .and_then(|n| i32::try_from(n).ok())
            .ok_or_else(|| {
                refused(format!(
                    "Cannot read monitor {field}. Check the host compositor and retry"
                ))
            })
    };
    let mut width = (f64::from(dimension("width")?) / scale) as i32;
    let mut height = (f64::from(dimension("height")?) / scale) as i32;
    if number(monitor, "transform") % 2 == 1 {
        std::mem::swap(&mut width, &mut height);
    }
    let mut reserved = [0; 4];
    if let Some(values) = monitor["reserved"].as_array() {
        for (target, value) in reserved.iter_mut().zip(values) {
            *target = value.as_i64().and_then(|n| n.try_into().ok()).unwrap_or(0);
        }
    }
    let outer = gaps(
        &json("j/getoption general:gaps_out").await?,
        DEFAULT_OUTER_GAP,
    );
    let inner = gaps(
        &json("j/getoption general:gaps_in").await?,
        DEFAULT_INNER_GAP,
    ) * 2;
    let slots = slots::layout(slots::Layout {
        width,
        height,
        reserved,
        outer,
        gap: inner,
        count,
    });
    let Some(mut slot) = slots.get(index.0).copied() else {
        return Err(refused(
            "The monitor has no room for this Session Card. Use a larger monitor or reduce the session cap",
        ));
    };
    if let Some(size) = size {
        let w = i32::try_from(size.width.get())
            .map_err(|_| refused("WindowSize width is too large. Reduce it and retry"))?;
        let h = i32::try_from(size.height.get())
            .map_err(|_| refused("WindowSize height is too large. Reduce it and retry"))?;
        if w > slot.width || h > slot.height {
            return Err(refused(
                "The Project WindowSize exceeds its Session Card slot. Reduce WindowSize or use a larger monitor",
            ));
        }
        slot.width = w;
        slot.height = h;
    }
    Ok(Placement {
        slot,
        origin: (number(monitor, "x"), number(monitor, "y")),
        monitor: monitor["name"].as_str().unwrap_or_default().into(),
    })
}
fn gaps(value: &Value, fallback: i32) -> i32 {
    value
        .get("int")
        .and_then(Value::as_i64)
        .and_then(|n| n.try_into().ok())
        .or_else(|| {
            value
                .get("custom")
                .and_then(Value::as_str)
                .and_then(|s| s.split_whitespace().next())
                .and_then(|s| s.parse().ok())
        })
        .unwrap_or(fallback)
}
fn selector(
    clients: &Value,
    event: Option<&hypr::HyprEvent>,
    pid: Option<WindowPid>,
    placed: &HashSet<String>,
) -> Option<(String, String)> {
    if let Some(pid) = pid {
        let client = clients
            .as_array()?
            .iter()
            .find(|client| client.get("pid").and_then(Value::as_u64) == Some(u64::from(pid.0)))?;
        let address = client.get("address")?.as_str()?.trim_start_matches("0x");
        if address.is_empty() || !address.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        return Some((address.to_owned(), format!("pid:{}", pid.0)));
    }
    if let Some(hypr::HyprEvent::OpenWindow { address, class, .. }) = event
        && class == "aquamarine"
        && !address.is_empty()
        && address.bytes().all(|byte| byte.is_ascii_hexdigit())
        && !placed.contains(address)
    {
        return Some((address.clone(), format!("address:0x{address}")));
    }
    None
}

static PLACED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

pub async fn place(
    pid: Option<WindowPid>,
    placement: Placement,
    events: Option<crate::compositor::WindowEvents>,
) -> Result<(), ProcessError> {
    let mut events = match events {
        Some(events) => events,
        None => hypr::subscribe().await?,
    };
    let mut slot = placement.slot;
    slot.x += placement.origin.0;
    slot.y += placement.origin.1;
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        place_mapped(pid, slot, &mut events),
    )
    .await
    .map_err(|_| {
        refused("The Session Window did not appear within 5 s. Check sandbox.err and retry")
    })?
}

async fn place_mapped(
    pid: Option<WindowPid>,
    slot: slots::Slot,
    events: &mut crate::compositor::WindowEvents,
) -> Result<(), ProcessError> {
    let mut event = None;
    loop {
        let clients = json("j/clients").await?;
        let selected = {
            let mut placed = PLACED
                .get_or_init(|| Mutex::new(HashSet::new()))
                .lock()
                .map_err(|_| refused("Cannot track Session Windows. Restart keek and retry"))?;
            if let Some(clients) = clients.as_array() {
                placed.retain(|address| {
                    clients.iter().any(|client| {
                        client
                            .get("address")
                            .and_then(Value::as_str)
                            .is_some_and(|value| value.trim_start_matches("0x") == address)
                    })
                });
            }
            let selected = selector(&clients, event.as_ref(), pid, &placed);
            if let Some((address, _)) = &selected {
                placed.insert(address.clone());
            }
            selected
        };
        if let Some((address, selector)) = selected {
            let result = dispatch(&selector, slot).await;
            if result.is_err()
                && let Ok(mut placed) = PLACED.get_or_init(|| Mutex::new(HashSet::new())).lock()
            {
                placed.remove(&address);
            }
            return result;
        }
        loop {
            let line = events
                .next_line()
                .await
                .map_err(|source| ProcessError::Io {
                    program: "Hyprland events".into(),
                    source,
                })?
                .ok_or_else(|| {
                    refused("Hyprland event stream closed before the Session Window appeared. Open again")
                })?;
            event = hypr::parse(&line);
            if matches!(&event, Some(hypr::HyprEvent::OpenWindow { class, .. }) if class == "aquamarine")
            {
                break;
            }
        }
    }
}

async fn dispatch(selector: &str, slot: slots::Slot) -> Result<(), ProcessError> {
    let slots::Slot {
        x,
        y,
        width,
        height,
    } = slot;
    // Resizing a floating window keeps its centre, so resize before the move.
    let lua = format!(
        "hl.dispatch(hl.dsp.window.move({{ window = '{selector}', workspace = 'name:keek', follow = false }})); hl.dispatch(hl.dsp.window.float({{ window = '{selector}', action = 'enable' }})); hl.dispatch(hl.dsp.window.resize({{ window = '{selector}', x = {width}, y = {height}, relative = false }})); hl.dispatch(hl.dsp.window.move({{ window = '{selector}', x = {x}, y = {y}, relative = false }}))"
    );
    let output = hypr::command(&format!("eval {lua}")).await?;
    if output.trim() == "ok" {
        Ok(())
    } else {
        Err(refused(format!(
            "Hyprland refused to place the Session Window: {}. Check `hyprctl eval` works on the host, then open the session again",
            output.trim()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn open(address: &str) -> hypr::HyprEvent {
        hypr::HyprEvent::OpenWindow {
            address: address.into(),
            workspace: "keek".into(),
            class: "aquamarine".into(),
        }
    }
    #[test]
    fn known_pid_wins_over_a_foreign_window_event() {
        let clients = json!([{ "pid": 7, "address": "0xabc" }, { "pid": 9, "address": "0xdef" }]);
        assert_eq!(
            selector(
                &clients,
                Some(&open("def")),
                Some(WindowPid(7)),
                &HashSet::new()
            ),
            Some(("abc".into(), "pid:7".into()))
        );
    }
    #[test]
    fn known_pid_ignores_foreign_aquamarine_windows() {
        assert_eq!(
            selector(
                &json!([{ "pid": 9, "address": "0xdef" }]),
                Some(&open("def")),
                Some(WindowPid(7)),
                &HashSet::new()
            ),
            None
        );
    }
    #[test]
    fn unknown_pid_accepts_the_first_new_window() {
        let placed = HashSet::from(["abc".into()]);
        assert_eq!(
            selector(&json!([]), Some(&open("abc")), None, &placed),
            None
        );
        assert_eq!(
            selector(&json!([]), Some(&open("def")), None, &placed),
            Some(("def".into(), "address:0xdef".into()))
        );
    }
    #[test]
    fn unknown_pid_rejects_empty_and_non_hex_addresses() {
        for address in ["", "bad!"] {
            assert_eq!(
                selector(&json!([]), Some(&open(address)), None, &HashSet::new()),
                None
            );
        }
    }
}
