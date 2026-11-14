#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HyprEvent {
    Submap(String),
    Workspace(String),
    FocusedMonitor(String),
    MoveWorkspace {
        workspace: String,
        monitor: String,
    },
    OpenWindow {
        address: String,
        workspace: String,
        class: String,
    },
    CloseWindow(String),
}
pub fn parse(line: &str) -> Option<HyprEvent> {
    let (name, data) = line.trim_end().split_once(">>")?;
    match name {
        "submap" => Some(HyprEvent::Submap(data.into())),
        "workspace" if !data.is_empty() => Some(HyprEvent::Workspace(data.into())),
        "workspacev2" => {
            let (_, name) = data.split_once(',')?;
            (!name.is_empty()).then(|| HyprEvent::Workspace(name.into()))
        }
        "focusedmon" | "focusedmonv2" => {
            let (monitor, _) = data.split_once(',')?;
            Some(HyprEvent::FocusedMonitor(monitor.into()))
        }
        "moveworkspace" => {
            let (workspace, monitor) = data.split_once(',')?;
            if workspace.is_empty() || monitor.is_empty() {
                return None;
            }
            Some(HyprEvent::MoveWorkspace {
                workspace: workspace.into(),
                monitor: monitor.into(),
            })
        }
        "moveworkspacev2" => {
            let mut fields = data.splitn(3, ',');
            let _id = fields.next()?;
            let workspace = fields.next()?;
            let monitor = fields.next()?;
            if workspace.is_empty() || monitor.is_empty() {
                return None;
            }
            Some(HyprEvent::MoveWorkspace {
                workspace: workspace.into(),
                monitor: monitor.into(),
            })
        }
        "openwindow" => {
            let mut fields = data.splitn(4, ',');
            let address = fields.next()?;
            if address.is_empty() {
                return None;
            }
            Some(HyprEvent::OpenWindow {
                address: address.into(),
                workspace: fields.next()?.into(),
                class: fields.next()?.into(),
            })
        }
        "closewindow" if !data.is_empty() => Some(HyprEvent::CloseWindow(data.into())),
        _ => None,
    }
}
use crate::process::ProcessError;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::net::UnixStream;
fn socket(name: &str) -> Result<std::path::PathBuf, ProcessError> {
    let missing = |key| ProcessError::Failed {
        program: "Hyprland IPC".into(),
        message: format!("{key} is missing. Start keek from Hyprland"),
    };
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").ok_or_else(|| missing("XDG_RUNTIME_DIR"))?;
    let signature = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")
        .ok_or_else(|| missing("HYPRLAND_INSTANCE_SIGNATURE"))?;
    socket_path(std::path::Path::new(&runtime), &signature, name)
}
fn socket_path(
    runtime: &std::path::Path,
    signature: &std::ffi::OsStr,
    name: &str,
) -> Result<std::path::PathBuf, ProcessError> {
    let path = runtime.join("hypr").join(signature).join(name);
    let length = path.as_os_str().len();
    if length >= 108 {
        return Err(ProcessError::Failed {
            program: "Hyprland IPC".into(),
            message: format!(
                "socket path {} is {length} bytes; the limit is 108 bytes including the terminator. Use a shorter XDG_RUNTIME_DIR",
                path.display()
            ),
        });
    }
    Ok(path)
}
async fn connect(name: &str) -> Result<UnixStream, ProcessError> {
    let path = socket(name)?;
    UnixStream::connect(&path)
        .await
        .map_err(|error| ProcessError::Failed {
            program: "Hyprland IPC".into(),
            message: format!(
                "cannot connect to {}: {error}. Check that Hyprland is running",
                path.display()
            ),
        })
}
fn io(source: std::io::Error) -> ProcessError {
    ProcessError::Io {
        program: "Hyprland IPC".into(),
        source,
    }
}
pub async fn command(request: &str) -> Result<String, ProcessError> {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let mut stream = connect(".socket.sock").await?;
        stream.write_all(request.as_bytes()).await.map_err(io)?;
        let mut reply = String::new();
        stream.read_to_string(&mut reply).await.map_err(io)?;
        Ok(reply)
    })
    .await
    .map_err(|_| ProcessError::Timeout {
        program: "Hyprland IPC".into(),
    })?
}
pub async fn subscribe() -> Result<Lines<BufReader<UnixStream>>, ProcessError> {
    Ok(BufReader::new(connect(".socket2.sock").await?).lines())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workspace_events_accept_both_versions() {
        assert_eq!(
            parse("workspacev2>>2,keek\n"),
            Some(HyprEvent::Workspace("keek".into()))
        );
        assert_eq!(
            parse("workspace>>code"),
            Some(HyprEvent::Workspace("code".into()))
        );
    }
    #[test]
    fn focused_monitor_events_keep_the_monitor() {
        assert_eq!(
            parse("focusedmonv2>>DP-1,2"),
            Some(HyprEvent::FocusedMonitor("DP-1".into()))
        );
        assert_eq!(
            parse("focusedmon>>DP-1,keek"),
            Some(HyprEvent::FocusedMonitor("DP-1".into()))
        );
    }
    #[test]
    fn moving_a_workspace_reports_its_monitor() {
        assert_eq!(
            parse("moveworkspace>>keek,DP-1"),
            Some(HyprEvent::MoveWorkspace {
                workspace: "keek".into(),
                monitor: "DP-1".into()
            })
        );
        assert_eq!(
            parse("moveworkspacev2>>3,keek,DP-1"),
            Some(HyprEvent::MoveWorkspace {
                workspace: "keek".into(),
                monitor: "DP-1".into()
            })
        );
    }
    #[test]
    fn window_titles_may_contain_commas() {
        assert_eq!(
            parse("openwindow>>abc,keek,aquamarine,title,comma"),
            Some(HyprEvent::OpenWindow {
                address: "abc".into(),
                workspace: "keek".into(),
                class: "aquamarine".into()
            })
        );
        assert_eq!(
            parse("closewindow>>abc"),
            Some(HyprEvent::CloseWindow("abc".into()))
        );
    }
    #[test]
    fn unrelated_or_malformed_events_are_ignored() {
        for line in [
            "",
            "workspacev2>>",
            "openwindow>>a,b",
            "openwindow>>,keek,aquamarine,title",
            "activewindow>>a,b",
            "focusedmon>>DP-1",
        ] {
            assert_eq!(parse(line), None);
        }
    }
}

#[cfg(test)]
mod socket_tests {
    use super::*;
    #[test]
    fn an_overlong_socket_path_names_the_path_and_its_byte_length() {
        let runtime = std::path::PathBuf::from(format!("/run/{}", "é".repeat(55)));
        let path = runtime.join("hypr/fixture/.socket.sock");
        let error = socket_path(&runtime, "fixture".as_ref(), ".socket.sock")
            .expect_err("overlong socket path must be refused")
            .to_string();
        assert!(error.contains(&path.display().to_string()), "{error}");
        assert!(
            error.contains(&path.as_os_str().len().to_string()),
            "{error}"
        );
        assert!(
            error.contains("108") && error.contains("shorter XDG_RUNTIME_DIR"),
            "{error}"
        );
    }
}
