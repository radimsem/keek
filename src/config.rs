use serde::Deserialize;
#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub session_cap: usize,
    pub decision_model: Option<DecisionModelConfig>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionModelConfig {
    pub endpoint: String,
    pub model: String,
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default = "default_threshold")]
    pub threshold: f64,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}
fn default_threshold() -> f64 {
    0.9
}
fn default_timeout_ms() -> u64 {
    1500
}
impl Default for Config {
    fn default() -> Self {
        Self {
            session_cap: 4,
            decision_model: None,
        }
    }
}
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error(
        "invalid keek config at {path}: session_cap must be greater than zero. Fix the file and restart keek."
    )]
    ZeroCap { path: std::path::PathBuf },
    #[error(
        "invalid keek config at {path}: decision_model.threshold is {value}, and it must be between 0 and 1. Fix the file and restart keek."
    )]
    Threshold {
        path: std::path::PathBuf,
        value: f64,
    },
    #[error(
        "invalid keek config at {path}: decision_model.endpoint is \"{endpoint}\", and it must start with http:// or https://. Fix the file and restart keek."
    )]
    Endpoint {
        path: std::path::PathBuf,
        endpoint: String,
    },
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
                if let Some(model) = &config.decision_model {
                    if !(0.0..=1.0).contains(&model.threshold) {
                        return Err(ConfigError::Threshold {
                            path,
                            value: model.threshold,
                        });
                    }
                    if !model.endpoint.starts_with("http://")
                        && !model.endpoint.starts_with("https://")
                    {
                        return Err(ConfigError::Endpoint {
                            path,
                            endpoint: model.endpoint.clone(),
                        });
                    }
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
    #[test]
    fn a_config_without_decision_model_leaves_it_off() {
        assert!(
            Config::parse("session_cap = 2")
                .unwrap()
                .decision_model
                .is_none()
        );
    }
    #[test]
    fn a_decision_model_table_parses_with_defaults() {
        let config = Config::parse(
            "[decision_model]\nendpoint = \"https://api.example.test\"\nmodel = \"jev-1\"",
        )
        .unwrap();
        let model = config.decision_model.unwrap();
        assert_eq!(model.endpoint, "https://api.example.test");
        assert_eq!(model.model, "jev-1");
        assert_eq!(model.api_key_env, None);
        assert_eq!(model.threshold, 0.9);
        assert_eq!(model.timeout_ms, 1500);
    }
    #[tokio::test]
    async fn a_threshold_outside_zero_to_one_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        for threshold in ["1.5", "-0.1", "nan"] {
            std::fs::write(
                &path,
                format!(
                    "[decision_model]\nendpoint = \"https://api.example.test\"\nmodel = \"jev-1\"\nthreshold = {threshold}"
                ),
            )
            .unwrap();
            let error = Config::read(&path).await.unwrap_err();
            assert!(matches!(error, ConfigError::Threshold { .. }), "{error}");
            let message = error.to_string();
            assert!(
                message.contains("config.toml") && message.contains("threshold"),
                "{message}"
            );
        }
    }
    #[tokio::test]
    async fn an_endpoint_that_is_not_http_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[decision_model]\nendpoint = \"api.typesafe.ai\"\nmodel = \"jev-1\"",
        )
        .unwrap();
        let error = Config::read(&path).await.unwrap_err();
        assert!(matches!(error, ConfigError::Endpoint { .. }), "{error}");
        let message = error.to_string();
        assert!(
            message.contains("config.toml") && message.contains("api.typesafe.ai"),
            "{message}"
        );
    }
    #[test]
    fn an_unknown_key_in_decision_model_is_refused() {
        assert!(
            Config::parse(
                "[decision_model]\nendpoint = \"https://api.example.test\"\nmodel = \"jev-1\"\napi_key = \"x\""
            )
            .is_err()
        );
    }
}
