use serde::{Deserialize, Serialize};
use std::num::{NonZeroU16, NonZeroU32};
use std::path::Path;
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(transparent)]
pub struct HostPort(pub NonZeroU16);
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WindowSize {
    pub width: NonZeroU32,
    pub height: NonZeroU32,
}
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub ports: Vec<HostPort>,
    pub window: Option<WindowSize>,
}
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("Cannot read Project settings at {path}: {source}. Fix the file and open again.")]
    Read {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("Invalid Project settings at {path}: {source}. Fix the file and open again.")]
    Parse {
        path: std::path::PathBuf,
        source: toml::de::Error,
    },
}
impl Settings {
    pub async fn read(path: &Path) -> Result<Self, SettingsError> {
        let path = path.to_owned();
        let read_path = path.clone();
        let text = tokio::task::spawn_blocking(move || std::fs::read_to_string(read_path))
            .await
            .map_err(|source| SettingsError::Read {
                path: path.clone(),
                source: std::io::Error::other(source),
            })?;
        match text {
            Ok(text) => {
                toml::from_str(&text).map_err(|source| SettingsError::Parse { path, source })
            }
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => Err(SettingsError::Read { path, source }),
        }
    }
    pub fn pasta_ports(&self) -> String {
        if self.ports.is_empty() {
            "none".into()
        } else {
            let mut ports: Vec<_> = self.ports.iter().map(|port| port.0).collect();
            ports.sort_unstable();
            ports.dedup();
            ports
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(",")
        }
    }
}
pub async fn read_project_settings(
    project: &str,
    directory: &Path,
) -> Result<(Settings, Vec<String>), crate::session::SessionError> {
    let settings_path = directory.join(format!("{project}.toml"));
    let settings = Settings::read(&settings_path)
        .await
        .map_err(|e| crate::session::SessionError::Refused(e.to_string()))?;
    let mut warnings = Vec::new();
    for (path, thing) in [
        (settings_path, "settings"),
        (directory.join(format!("{project}.env")), "Test Credentials"),
    ] {
        let check = path.clone();
        let exists = tokio::task::spawn_blocking(move || check.exists())
            .await
            .map_err(|e| {
                crate::session::SessionError::Refused(format!(
                    "Cannot check Project files: {e}. Open again."
                ))
            })?;
        if !exists {
            warnings.push(format!("Project {project} has no {thing} at {}. If the Project name is misspelled, open again with the right name. Otherwise create the file.", path.display()));
        }
    }
    Ok((settings, warnings))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pasta_ports_are_sorted_and_deduplicated() {
        let settings: Settings = toml::from_str("ports = [6379, 5432, 5432]").unwrap();
        assert_eq!(settings.pasta_ports(), "5432,6379");
    }
    #[tokio::test]
    async fn project_settings_default_when_the_file_is_missing_or_empty() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("demo.toml");
        assert!(Settings::read(&p).await.unwrap().window.is_none());
        std::fs::write(&p, "").unwrap();
        assert_eq!(Settings::read(&p).await.unwrap().pasta_ports(), "none");
    }
    #[tokio::test]
    async fn project_settings_read_configured_ports_and_window() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("demo.toml");
        std::fs::write(
            &p,
            "ports = [5432,6379]\n[window]\nwidth = 390\nheight = 844\n",
        )
        .unwrap();
        let s = Settings::read(&p).await.unwrap();
        assert_eq!(s.pasta_ports(), "5432,6379");
        assert_eq!(s.window.unwrap().width.get(), 390);
    }
    #[tokio::test]
    async fn project_settings_refuse_invalid_values_and_name_the_file() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("demo.toml");
        for text in [
            "unknown = true",
            "ports = [0]",
            "[window]\nwidth=0\nheight=844",
        ] {
            std::fs::write(&p, text).unwrap();
            let e = Settings::read(&p).await.unwrap_err().to_string();
            assert!(e.contains(p.to_str().unwrap()), "{e}");
        }
    }
}
