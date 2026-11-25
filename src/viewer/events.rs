use super::{ShowTarget, rail_view::ShowKeys};
use crate::hypr::{self, HyprEvent};
use std::collections::HashSet;
use tokio::sync::mpsc;

pub async fn watch(
    sender: mpsc::Sender<HashSet<String>>,
    shows: mpsc::Sender<ShowTarget>,
    keys: mpsc::Sender<ShowKeys>,
) {
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
        // Rail cards show only on `keek`, so a bind added elsewhere shows once the developer switches back.
        match hypr::command("j/binds").await {
            Ok(json) => match serde_json::from_str::<serde_json::Value>(&json) {
                Ok(binds) => {
                    if keys.send(ShowKeys::from_binds(&binds)).await.is_err() {
                        return;
                    }
                }
                Err(error) => tracing::warn!(%error, "Cannot parse Hyprland binds"),
            },
            Err(error) => tracing::warn!(%error, "Cannot query Hyprland binds"),
        }
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => match hypr::parse(&line) {
                    Some(
                        HyprEvent::Workspace(_)
                        | HyprEvent::FocusedMonitor(_)
                        | HyprEvent::MoveWorkspace { .. },
                    ) => break,
                    Some(event @ HyprEvent::Custom(_)) => {
                        if let Some(target) = show_target(&event)
                            && let Err(mpsc::error::TrySendError::Full(_)) = shows.try_send(target)
                        {
                            tracing::debug!("Dropped a show event because the channel is full");
                        }
                    }
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

pub fn show_target(event: &HyprEvent) -> Option<ShowTarget> {
    let HyprEvent::Custom(data) = event else {
        return None;
    };
    let target = data.strip_prefix("keek-show ")?;
    if target.trim().is_empty() {
        return None;
    }
    match target.parse::<usize>() {
        Ok(number) => (number > 0).then_some(ShowTarget::Number(number)),
        Err(_) => target.parse().ok().map(ShowTarget::Session),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::viewer::ShowTarget;
    #[test]
    fn the_show_event_names_a_shortcut_number_or_a_session() {
        assert_eq!(
            show_target(&HyprEvent::Custom("keek-show 2".into())),
            Some(ShowTarget::Number(2))
        );
        assert_eq!(
            show_target(&HyprEvent::Custom("keek-show billing-12-00-00".into())),
            Some(ShowTarget::Session("billing-12-00-00".parse().unwrap()))
        );
        for data in ["keek-show", "keek-show ", "other 2", "keek-shows 2"] {
            assert_eq!(show_target(&HyprEvent::Custom(data.into())), None, "{data}");
        }
        assert_eq!(show_target(&HyprEvent::Submap("keek-show 2".into())), None);
    }
}
