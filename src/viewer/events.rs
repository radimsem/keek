use crate::hypr::{self, HyprEvent};
use std::collections::HashSet;
use tokio::sync::mpsc;

pub async fn watch(sender: mpsc::Sender<HashSet<String>>) {
    let mut lines = match hypr::subscribe().await {
        Ok(lines) => lines,
        Err(error) => {
            tracing::warn!(%error, "Cannot subscribe to Hyprland events; Session Footer visibility will not update");
            return;
        }
    };
    loop {
        match hypr::command("j/monitors").await {
            Ok(json) => match serde_json::from_str::<serde_json::Value>(&json) {
                Ok(monitors) => {
                    let names = monitors
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|monitor| {
                            monitor
                                .pointer("/activeWorkspace/name")
                                .and_then(serde_json::Value::as_str)
                                == Some("keek")
                        })
                        .filter_map(|monitor| {
                            monitor
                                .get("name")
                                .and_then(serde_json::Value::as_str)
                                .map(str::to_owned)
                        })
                        .collect();
                    if sender.send(names).await.is_err() {
                        return;
                    }
                }
                Err(error) => tracing::warn!(%error, "Cannot parse Hyprland monitors"),
            },
            Err(error) => tracing::warn!(%error, "Cannot query Hyprland monitors"),
        }
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => match hypr::parse(&line) {
                    Some(
                        HyprEvent::Workspace(_)
                        | HyprEvent::FocusedMonitor(_)
                        | HyprEvent::MoveWorkspace { .. },
                    ) => break,
                    Some(
                        HyprEvent::Submap(_)
                        | HyprEvent::OpenWindow { .. }
                        | HyprEvent::CloseWindow(_),
                    )
                    | None => {}
                },
                Ok(None) => {
                    tracing::warn!(
                        "Hyprland event stream ended; Session Footer visibility will not update"
                    );
                    return;
                }
                Err(error) => {
                    tracing::warn!(%error, "Hyprland event stream failed; Session Footer visibility will not update");
                    return;
                }
            }
        }
    }
}
