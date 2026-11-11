use super::{Profile, ProjectError, ProjectFiles};
use crate::toolchain::Toolchains;
use sha2::{Digest, Sha256};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectKey(pub String);

const LOCKFILES: &[&str] = &[
    "package-lock.json",
    "npm-shrinkwrap.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "bun.lock",
    "bun.lockb",
    "deno.lock",
    "Cargo.lock",
    "poetry.lock",
    "uv.lock",
    "pdm.lock",
    "Pipfile.lock",
    "Gemfile.lock",
    "composer.lock",
    "go.sum",
    "flake.lock",
    "mix.lock",
    "pubspec.lock",
    "Podfile.lock",
    "packages.lock.json",
    "gradle.lockfile",
];

impl ProjectFiles {
    /// Covers what builds the Project Layer: the install command, the resolved toolchain versions, the environment and the tracked lockfiles.
    pub fn key(
        &self,
        worktree: &Path,
        profile: &Profile,
        toolchains: &Toolchains,
    ) -> Result<ProjectKey, ProjectError> {
        let mut hash = Sha256::new();
        frame(&mut hash, b"install");
        frame(&mut hash, profile.install.as_bytes());
        frame(&mut hash, b"toolchain");
        for resolved in &toolchains.tools {
            frame(&mut hash, resolved.tool.name().as_bytes());
            frame(&mut hash, resolved.version.as_bytes());
        }
        frame(&mut hash, b"env");
        for (name, value) in &profile.env {
            frame(&mut hash, name.as_bytes());
            frame(&mut hash, value.as_bytes());
        }
        frame(&mut hash, b"lockfiles");
        for path in &self.tracked {
            if path
                .file_name()
                .is_some_and(|name| LOCKFILES.iter().any(|candidate| name == *candidate))
            {
                frame(&mut hash, path.as_os_str().as_bytes());
                hash_file(&mut hash, &worktree.join(path))?;
            }
        }
        Ok(ProjectKey(
            hash.finalize()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
        ))
    }
}

fn frame(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}

fn hash_file(hash: &mut Sha256, path: &Path) -> Result<(), ProjectError> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.is_file() => {
            frame(hash, b"file");
            frame(hash, &std::fs::read(path)?);
        }
        Ok(m) if m.is_symlink() => {
            frame(hash, b"link");
            frame(hash, std::fs::read_link(path)?.as_os_str().as_bytes());
        }
        Ok(_) => frame(hash, b"other"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => frame(hash, b"missing"),
        Err(e) => return Err(e.into()),
    };
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::HostRunner;
    use crate::toolchain::{Fetch, ResolvedTool, Tool};
    use std::os::unix::fs::symlink;

    fn git(directory: &Path, args: &[&str]) {
        assert!(
            std::process::Command::new("git")
                .args(args)
                .current_dir(directory)
                .output()
                .unwrap()
                .status
                .success()
        );
    }

    fn repository() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        git(d.path(), &["init", "-q"]);
        d
    }

    fn profile(text: &str) -> Profile {
        Profile::parse(Path::new("demo.toml"), text).unwrap()
    }

    async fn key(root: &Path, profile: &Profile) -> ProjectKey {
        key_with(root, profile, &Toolchains::default()).await
    }

    async fn key_with(root: &Path, profile: &Profile, toolchains: &Toolchains) -> ProjectKey {
        ProjectFiles::read(&HostRunner, root)
            .await
            .unwrap()
            .key(root, profile, toolchains)
            .unwrap()
    }

    fn resolved(tool: Tool, version: &str) -> Toolchains {
        Toolchains {
            store: "/store".into(),
            tools: vec![ResolvedTool {
                tool,
                pin: "3".into(),
                version: version.into(),
                bin_dirs: vec!["/store/bin".into()],
                fetch: Fetch::Present,
            }],
        }
    }

    #[tokio::test]
    async fn project_key_changes_when_a_lockfile_in_a_submodule_changes() {
        let directory = crate::project::submodules::tests::repository_with_submodule();
        let root = directory.path().join("super");
        let profile = profile("install = \"make\"\nstart = \"run\"");
        let first = key(&root, &profile).await;
        std::fs::write(root.join("vendor/lib/Cargo.lock"), "second").unwrap();
        assert_ne!(first, key(&root, &profile).await);
    }

    #[tokio::test]
    async fn the_key_follows_the_resolved_toolchain_version_not_the_pin() {
        let d = repository();
        let root = d.path();
        let pinned = profile("start = \"run\"\n[toolchain]\npython = \"3\"");
        let old = key_with(root, &pinned, &resolved(Tool::Python, "3.12.1")).await;
        let newer = key_with(root, &pinned, &resolved(Tool::Python, "3.13.0")).await;
        assert_ne!(old, newer);
        assert_ne!(
            key_with(root, &pinned, &resolved(Tool::Node, "3.12.1")).await,
            old
        );
        assert_eq!(
            key_with(
                root,
                &profile("start = \"run\""),
                &resolved(Tool::Python, "3.12.1")
            )
            .await,
            old
        );
    }

    #[tokio::test]
    async fn the_key_changes_with_install_env_and_tracked_lockfiles() {
        let d = repository();
        let root = d.path();
        std::fs::write(root.join("Cargo.lock"), "first").unwrap();
        git(root, &["add", "Cargo.lock"]);
        let base = profile("install = \"make\"\nstart = \"run\"\n[env]\nAPP_ENV = \"test\"");
        let first = key(root, &base).await;
        assert_eq!(first, key(root, &base).await);
        for changed in [
            "install = \"make all\"\nstart = \"run\"\n[env]\nAPP_ENV = \"test\"",
            "install = \"make\"\nstart = \"run\"\n[env]\nAPP_ENV = \"other\"",
            "install = \"make\"\nstart = \"run\"\n[env]\nAPP_ENV = \"test\"\nMORE = \"x\"",
        ] {
            assert_ne!(first, key(root, &profile(changed)).await, "{changed}");
        }
        std::fs::write(root.join("Cargo.lock"), "second").unwrap();
        assert_ne!(first, key(root, &base).await);
    }

    #[tokio::test]
    async fn the_key_ignores_start_ports_window_and_untracked_lockfiles() {
        let d = repository();
        let root = d.path();
        let base = profile("install = \"make\"\nstart = \"run\"");
        let first = key(root, &base).await;
        std::fs::write(root.join("uv.lock"), "untracked").unwrap();
        let other = profile(
            "install = \"make\"\nstart = \"other\"\nports = [5432]\n[window]\nwidth = 390\nheight = 844",
        );
        assert_eq!(first, key(root, &other).await);
    }

    #[tokio::test]
    async fn the_key_frames_fields_so_moving_bytes_changes_the_hash() {
        let d = repository();
        let root = d.path();
        std::fs::write(root.join("Cargo.lock"), b"c").unwrap();
        git(root, &["add", "Cargo.lock"]);
        let first = key(root, &profile("install = \"ab\"\nstart = \"x\"")).await;
        std::fs::write(root.join("Cargo.lock"), b"bc").unwrap();
        assert_ne!(
            first,
            key(root, &profile("install = \"a\"\nstart = \"x\"")).await
        );
    }

    #[tokio::test]
    async fn the_key_hashes_lockfile_symlinks_without_following_them() {
        let d = repository();
        let root = d.path();
        symlink("absent-one", root.join("uv.lock")).unwrap();
        git(root, &["add", "uv.lock"]);
        let base = profile("start = \"x\"");
        let first = key(root, &base).await;
        std::fs::remove_file(root.join("uv.lock")).unwrap();
        symlink("absent-two", root.join("uv.lock")).unwrap();
        assert_ne!(first, key(root, &base).await);
    }
}
