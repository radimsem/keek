use std::ffi::OsString;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    runtime_dir: PathBuf,
    state_dir: PathBuf,
    sessions_dir: PathBuf,
    config_file: PathBuf,
    projects_config: PathBuf,
    projects_cache: PathBuf,
    trees: PathBuf,
    toolchains: PathBuf,
    prepare: PathBuf,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PathsError {
    #[error(
        "XDG_RUNTIME_DIR is not set, so keek has nowhere to put its daemon socket. Set it to your user runtime folder, normally /run/user/<uid>."
    )]
    NoRuntimeDir,
    #[error(
        "HOME is not set, so keek can't find its state and cache folders. Set HOME, or set XDG_STATE_HOME and XDG_CACHE_HOME."
    )]
    NoHome,
}

impl Paths {
    pub fn from_env() -> Result<Self, PathsError> {
        Self::from_vars(|name| std::env::var_os(name))
    }

    #[cfg(test)]
    pub(crate) fn under_root(root: &Path) -> Self {
        Self {
            runtime_dir: root.join("run/keek"),
            state_dir: root.join("state/keek"),
            sessions_dir: root.join("cache/keek/sessions"),
            config_file: root.join("config/keek/config.toml"),
            projects_config: root.join("config/keek/projects"),
            projects_cache: root.join("cache/keek/projects"),
            trees: root.join("cache/keek/trees"),
            toolchains: root.join("cache/keek/toolchains"),
            prepare: root.join("cache/keek/prepare"),
        }
    }

    fn from_vars(var: impl Fn(&str) -> Option<OsString>) -> Result<Self, PathsError> {
        // The XDG spec says to ignore a variable that isn't an absolute path.
        let absolute = |name: &str| {
            var(name)
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
        };
        let or_under_home = |name: &str, fallback: &str| {
            absolute(name)
                .or_else(|| absolute("HOME").map(|home| home.join(fallback)))
                .ok_or(PathsError::NoHome)
        };

        Ok(Self {
            runtime_dir: absolute("XDG_RUNTIME_DIR")
                .ok_or(PathsError::NoRuntimeDir)?
                .join("keek"),
            state_dir: or_under_home("XDG_STATE_HOME", ".local/state")?.join("keek"),
            sessions_dir: or_under_home("XDG_CACHE_HOME", ".cache")?.join("keek/sessions"),
            projects_config: or_under_home("XDG_CONFIG_HOME", ".config")?.join("keek/projects"),
            projects_cache: or_under_home("XDG_CACHE_HOME", ".cache")?.join("keek/projects"),
            trees: or_under_home("XDG_CACHE_HOME", ".cache")?.join("keek/trees"),
            toolchains: or_under_home("XDG_CACHE_HOME", ".cache")?.join("keek/toolchains"),
            // Outside projects/<project>/, whose old Project Layers keek removes.
            prepare: or_under_home("XDG_CACHE_HOME", ".cache")?.join("keek/prepare"),
            config_file: or_under_home("XDG_CONFIG_HOME", ".config")?.join("keek/config.toml"),
        })
    }

    pub fn projects_config(&self) -> &Path {
        &self.projects_config
    }
    pub fn projects_cache(&self) -> &Path {
        &self.projects_cache
    }
    pub fn trees(&self) -> &Path {
        &self.trees
    }
    pub fn toolchains(&self) -> &Path {
        &self.toolchains
    }
    pub fn prepare(&self) -> &Path {
        &self.prepare
    }

    pub fn config_file(&self) -> &Path {
        &self.config_file
    }

    #[must_use]
    pub fn runtime_dir(&self) -> &Path {
        &self.runtime_dir
    }

    #[must_use]
    pub fn socket(&self) -> PathBuf {
        self.runtime_dir.join("daemon.sock")
    }

    #[must_use]
    pub fn lock(&self) -> PathBuf {
        self.runtime_dir.join("daemon.lock")
    }

    #[must_use]
    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    #[must_use]
    pub fn daemon_log(&self) -> PathBuf {
        self.state_dir.join("daemon.log")
    }

    #[must_use]
    pub fn sessions_dir(&self) -> &Path {
        &self.sessions_dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths_from(vars: &[(&str, &str)]) -> Result<Paths, PathsError> {
        Paths::from_vars(|name| {
            vars.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        })
    }

    #[test]
    fn folders_default_to_xdg_locations_under_home() {
        let paths =
            paths_from(&[("XDG_RUNTIME_DIR", "/run/user/1000"), ("HOME", "/home/me")]).unwrap();

        assert_eq!(paths.socket(), Path::new("/run/user/1000/keek/daemon.sock"));
        assert_eq!(paths.lock(), Path::new("/run/user/1000/keek/daemon.lock"));
        assert_eq!(
            paths.daemon_log(),
            Path::new("/home/me/.local/state/keek/daemon.log")
        );
        assert_eq!(
            paths.sessions_dir(),
            Path::new("/home/me/.cache/keek/sessions")
        );
    }

    #[test]
    fn xdg_variables_override_the_home_defaults() {
        let paths = paths_from(&[
            ("XDG_RUNTIME_DIR", "/run/user/1000"),
            ("XDG_STATE_HOME", "/state"),
            ("XDG_CACHE_HOME", "/cache"),
            ("XDG_CONFIG_HOME", "/config"),
        ])
        .unwrap();

        assert_eq!(paths.daemon_log(), Path::new("/state/keek/daemon.log"));
        assert_eq!(paths.sessions_dir(), Path::new("/cache/keek/sessions"));
        assert_eq!(paths.projects_config(), Path::new("/config/keek/projects"));
        assert_eq!(paths.projects_cache(), Path::new("/cache/keek/projects"));
        assert_eq!(paths.trees(), Path::new("/cache/keek/trees"));
        assert_eq!(paths.toolchains(), Path::new("/cache/keek/toolchains"));
        assert_eq!(paths.prepare(), Path::new("/cache/keek/prepare"));
    }

    #[test]
    fn a_relative_or_empty_variable_is_ignored() {
        let paths = paths_from(&[
            ("XDG_RUNTIME_DIR", "/run/user/1000"),
            ("XDG_STATE_HOME", "relative"),
            ("XDG_CACHE_HOME", ""),
            ("HOME", "/home/me"),
        ])
        .unwrap();

        assert_eq!(paths.state_dir(), Path::new("/home/me/.local/state/keek"));
        assert_eq!(
            paths.sessions_dir(),
            Path::new("/home/me/.cache/keek/sessions")
        );
    }

    #[test]
    fn a_missing_runtime_dir_or_home_is_an_error() {
        assert_eq!(
            paths_from(&[("HOME", "/home/me")]),
            Err(PathsError::NoRuntimeDir)
        );
        assert_eq!(
            paths_from(&[("XDG_RUNTIME_DIR", "/run/user/1000")]),
            Err(PathsError::NoHome)
        );
    }
}
