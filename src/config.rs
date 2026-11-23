use serde::Deserialize;
#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub session_cap: usize,
}
impl Default for Config {
    fn default() -> Self {
        Self { session_cap: 4 }
    }
}
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error(
        "invalid keek config at {path}: session_cap must be greater than zero. Fix the file and restart keek."
    )]
    ZeroCap { path: std::path::PathBuf },
    #[error("could not read keek config at {path}: {source}. Fix the file and restart keek.")]
    Read {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("invalid keek config at {path}: {source}. Fix the file and restart keek.")]
    Parse {
        path: std::path::PathBuf,
        source: toml::de::Error,
    },
}
impl Config {
    pub fn parse(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(text)
    }
    pub async fn read(path: &std::path::Path) -> Result<Self, ConfigError> {
        let path = path.to_owned();
        let read_path = path.clone();
        let text = tokio::task::spawn_blocking(move || std::fs::read_to_string(read_path))
            .await
            .map_err(|error| ConfigError::Read {
                path: path.clone(),
                source: std::io::Error::other(error),
            })?;
        match text {
            Ok(text) => {
                let config = Self::parse(&text).map_err(|source| ConfigError::Parse {
                    path: path.clone(),
                    source,
                })?;
                if config.session_cap == 0 {
                    return Err(ConfigError::ZeroCap { path });
                }
                Ok(config)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => Err(ConfigError::Read { path, source }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn session_cap_defaults_to_four() {
        assert_eq!(Config::parse("").unwrap().session_cap, 4);
    }
    #[test]
    fn session_cap_reads_the_configured_value() {
        assert_eq!(Config::parse("session_cap = 3").unwrap().session_cap, 3);
    }
    #[tokio::test]
    async fn config_read_defaults_when_missing_and_names_invalid_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        assert_eq!(Config::read(&path).await.unwrap().session_cap, 4);
        std::fs::write(&path, "bad = 3").unwrap();
        assert!(
            Config::read(&path)
                .await
                .unwrap_err()
                .to_string()
                .contains("config.toml")
        );
        std::fs::write(&path, "session_cap = 0").unwrap();
        let error = Config::read(&path).await.unwrap_err().to_string();
        assert!(
            error.contains("config.toml") && error.contains("session_cap"),
            "{error}"
        );
    }
    #[test]
    fn unknown_keys_are_refused() {
        assert!(Config::parse("sessions = 3").is_err());
    }
}
