use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::num::{NonZeroU16, NonZeroU32};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(transparent)]
pub struct HostPort(pub NonZeroU16);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WindowSize {
    pub width: NonZeroU32,
    pub height: NonZeroU32,
}

// Field docs become the descriptions in set_project_profile's schema.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    /// Shell command run from the worktree root inside the install sandbox. Its writes become the Project Layer. Leave it out for an empty Project Layer.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub install: String,
    /// Shell command run from the worktree root that starts the App Under Test.
    pub start: String,
    /// Pinned toolchain versions that keek installs into the Toolchain Store and puts on PATH for install and start.
    #[serde(default, skip_serializing_if = "crate::toolchain::Toolchain::is_empty")]
    pub toolchain: crate::toolchain::Toolchain,
    /// Non-secret environment variables for both install and start. The Verifier can see them.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// Host loopback ports forwarded into the install and session sandboxes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ports: Vec<HostPort>,
    /// Session Window size. Without it the window is 1280x800.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<WindowSize>,
}

#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error(
        "Project {project} has no Project Profile at {path}. If the Project name is misspelled, use the right name. Otherwise the coding agent writes one with the `keek-setup` skill and set_project_profile, then tries again. A Verifier stops and reports this refusal to the coding agent. If the skill is not installed, ask the developer to install it from https://github.com/radimsem/keek#install-the-setup-skill.",
        path = .path.display()
    )]
    Missing { project: String, path: PathBuf },
    #[error("Cannot read the Project Profile at {path}: {source}. Check the file and try again.", path = .path.display())]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("Invalid Project Profile at {path}: {source}. Fix it with set_project_profile, then try again.", path = .path.display())]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("Invalid Project Profile at {path}: {reason}. Fix it with set_project_profile, then try again.", path = .path.display())]
    Invalid { path: PathBuf, reason: String },
    #[error("Cannot write the Project Profile at {path}: {source}. Check the folder's permissions and try again.", path = .path.display())]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
}

// keek sets these itself, so a profile value would break the sandbox or the desktop.
const RESERVED: &[&str] = &[
    "HOME",
    "USER",
    "LANG",
    "PATH",
    "OMARCHY_PATH",
    "DISPLAY",
    "ATSPI_DBUS_IMPLEMENTATION",
    "HYPRLAND_INSTANCE_SIGNATURE",
    "DBUS_SESSION_BUS_ADDRESS",
];
const RESERVED_PREFIXES: &[&str] = &["KEEK_", "XDG_", "WAYLAND_", "CUA_DRIVER_"];

impl Profile {
    /// The first rule the profile breaks, as a sentence fragment.
    pub fn problem(&self) -> Option<String> {
        if self.start.trim().is_empty() {
            return Some("start is empty. Give the command that starts the App Under Test".into());
        }
        for (field, value) in [("install", &self.install), ("start", &self.start)] {
            if value.contains('\0') {
                return Some(format!("{field} contains a NUL byte. Remove it"));
            }
        }
        if let Some(problem) = self.toolchain.problem() {
            return Some(problem);
        }
        for (name, value) in &self.env {
            if !crate::credentials::valid_name(name) {
                return Some(format!(
                    "env name {name:?} is not valid. Use letters, digits and _, not starting with a digit"
                ));
            }
            if RESERVED.contains(&name.as_str())
                || RESERVED_PREFIXES
                    .iter()
                    .any(|prefix| name.starts_with(prefix))
            {
                return Some(format!(
                    "env name {name} is set by keek. Remove it from env"
                ));
            }
            if value.contains('\0') {
                return Some(format!(
                    "env value of {name} contains a NUL byte. Remove it"
                ));
            }
        }
        None
    }

    pub fn parse(path: &Path, text: &str) -> Result<Self, ProfileError> {
        let profile: Self = toml::from_str(text).map_err(|source| ProfileError::Parse {
            path: path.to_owned(),
            source,
        })?;
        match profile.problem() {
            Some(reason) => Err(ProfileError::Invalid {
                path: path.to_owned(),
                reason,
            }),
            None => Ok(profile),
        }
    }

    pub fn read(project: &str, path: &Path) -> Result<Self, ProfileError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(path, &text),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                Err(ProfileError::Missing {
                    project: project.to_owned(),
                    path: path.to_owned(),
                })
            }
            Err(source) => Err(ProfileError::Read {
                path: path.to_owned(),
                source,
            }),
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

pub fn profile_path(directory: &Path, project: &str) -> PathBuf {
    directory.join(format!("{project}.toml"))
}

/// Reads the Project Profile and warns when the Project has no Test Credentials.
pub async fn read_project_profile(
    project: &str,
    directory: &Path,
) -> Result<(Profile, Vec<String>), crate::session::SessionError> {
    let refused = |error: String| crate::session::SessionError::Refused(error);
    let path = profile_path(directory, project);
    let credentials = directory.join(format!("{project}.env"));
    let name = project.to_owned();
    let (profile, has_credentials) = tokio::task::spawn_blocking(move || {
        Profile::read(&name, &path).map(|profile| (profile, credentials.exists()))
    })
    .await
    .map_err(|e| refused(format!("Cannot read the Project Profile: {e}. Try again.")))?
    .map_err(|e| refused(e.to_string()))?;
    let mut warnings = Vec::new();
    if !has_credentials {
        warnings.push(format!("Project {project} has no Test Credentials at {}. If the Project name is misspelled, open again with the right name. Otherwise create the file.", directory.join(format!("{project}.env")).display()));
    }
    Ok((profile, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<Profile, ProfileError> {
        Profile::parse(Path::new("/config/demo.toml"), text)
    }

    #[test]
    fn a_profile_reads_its_commands_env_ports_and_window() {
        let profile = parse(
            "install = \"npm ci\"\nstart = \"npm run dev\"\nports = [5432]\n[env]\nAPP_ENV = \"test\"\n[window]\nwidth = 390\nheight = 844\n",
        )
        .unwrap();
        assert_eq!(profile.install, "npm ci");
        assert_eq!(profile.start, "npm run dev");
        assert_eq!(profile.env["APP_ENV"], "test");
        assert_eq!(profile.pasta_ports(), "5432");
        assert_eq!(profile.window.unwrap().width.get(), 390);
    }

    #[test]
    fn install_is_optional_and_start_is_required() {
        assert!(parse("start = \"./run\"").unwrap().install.is_empty());
        assert!(parse("install = \"make\"").is_err());
        assert!(
            parse("start = \"  \"")
                .unwrap_err()
                .to_string()
                .contains("start is empty")
        );
    }

    #[test]
    fn a_profile_refuses_unknown_keys_and_bad_values_and_names_the_file() {
        for text in [
            "start = \"x\"\nunknown = true",
            "start = \"x\"\nports = [0]",
            "start = \"x\"\n[window]\nwidth = 0\nheight = 844",
            "start = \"x\"\n[env]\n\"1BAD\" = \"v\"",
        ] {
            let error = parse(text).unwrap_err().to_string();
            assert!(error.contains("/config/demo.toml"), "{error}");
            assert!(error.contains("set_project_profile"), "{error}");
        }
    }

    #[test]
    fn env_refuses_the_names_keek_sets() {
        for name in [
            "PATH",
            "HOME",
            "KEEK_PROJECT",
            "XDG_RUNTIME_DIR",
            "WAYLAND_SOCKET",
        ] {
            let error = parse(&format!("start = \"x\"\n[env]\n{name} = \"v\""))
                .unwrap_err()
                .to_string();
            assert!(error.contains(&format!("{name} is set by keek")), "{error}");
        }
    }

    #[test]
    fn a_missing_profile_names_the_setup_skill_and_where_to_install_it() {
        let directory = tempfile::tempdir().unwrap();
        let path = profile_path(directory.path(), "demo");
        let error = Profile::read("demo", &path).unwrap_err().to_string();
        assert!(error.contains("`keek-setup` skill"), "{error}");
        assert!(
            error.contains("https://github.com/radimsem/keek#install-the-setup-skill"),
            "{error}"
        );
        assert!(error.contains("A Verifier stops"), "{error}");
        assert!(error.contains(path.to_str().unwrap()), "{error}");
    }

    #[test]
    fn pasta_ports_are_sorted_and_deduplicated() {
        let profile = parse("start = \"x\"\nports = [6379, 5432, 5432]").unwrap();
        assert_eq!(profile.pasta_ports(), "5432,6379");
    }
}
