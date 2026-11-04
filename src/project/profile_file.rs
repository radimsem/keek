use super::{Profile, ProfileError, profile_path};
use std::path::{Path, PathBuf};

pub struct WrittenProfile {
    pub path: PathBuf,
    pub toml: String,
}

// Two coding agents can set the same Project's profile at once, and each write moves the .bak.
static WRITES: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Checks `profile`, the JSON a coding agent sent, without writing anything.
pub fn check_profile(path: &Path, profile: serde_json::Value) -> Result<Profile, ProfileError> {
    // Going through TOML gives the same parser, and its errors name the key, as a hand-edited file gets.
    let text = toml::to_string(&profile).map_err(|error| ProfileError::Invalid {
        path: path.to_owned(),
        reason: format!("it is not a TOML table: {error}"),
    })?;
    Profile::parse(path, &text)
}

/// Replaces the Project Profile whole, keeping the previous one as `<project>.toml.bak`.
pub fn write_profile(
    directory: &Path,
    project: &str,
    profile: &Profile,
) -> Result<WrittenProfile, ProfileError> {
    use std::os::unix::fs::DirBuilderExt;
    let path = profile_path(directory, project);
    let toml = toml::to_string(profile).map_err(|error| ProfileError::Invalid {
        path: path.clone(),
        reason: error.to_string(),
    })?;
    let failed = |source| ProfileError::Write {
        path: path.clone(),
        source,
    };
    let _writing = WRITES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(directory)
        .map_err(failed)?;
    let temporary = directory.join(format!("{project}.toml.tmp"));
    std::fs::write(&temporary, &toml).map_err(failed)?;
    match std::fs::copy(&path, directory.join(format!("{project}.toml.bak"))) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(failed(error)),
    }
    std::fs::rename(&temporary, &path).map_err(failed)?;
    Ok(WrittenProfile { path, toml })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn write(directory: &Path, value: serde_json::Value) -> Result<WrittenProfile, ProfileError> {
        let profile = check_profile(&profile_path(directory, "demo"), value)?;
        write_profile(directory, "demo", &profile)
    }

    #[test]
    fn an_invalid_profile_is_refused_with_the_reason_and_nothing_is_written() {
        let directory = tempfile::tempdir().unwrap();
        for (value, reason) in [
            (json!({"install":"make"}), "start"),
            (json!({"start":"run","unknown":1}), "unknown"),
            (
                json!({"start":"run","env":{"PATH":"/x"}}),
                "PATH is set by keek",
            ),
            (json!({"start":"run","ports":[0]}), "ports"),
        ] {
            let error = write(directory.path(), value).err().unwrap().to_string();
            assert!(error.contains(reason), "{reason}: {error}");
        }
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }

    #[test]
    fn a_valid_profile_replaces_the_previous_one_and_keeps_one_bak() {
        let directory = tempfile::tempdir().unwrap();
        let first = write(
            directory.path(),
            json!({"install":"npm ci","start":"npm start"}),
        )
        .unwrap();
        assert_eq!(first.path, directory.path().join("demo.toml"));
        let second = write(directory.path(), json!({"start":"npm run dev"})).unwrap();
        write(directory.path(), json!({"start":"npm run dev -- --host"})).unwrap();
        let backup = std::fs::read_to_string(directory.path().join("demo.toml.bak")).unwrap();
        assert_eq!(backup, second.toml);
        let mut names: Vec<_> = std::fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names, ["demo.toml", "demo.toml.bak"]);
    }

    #[test]
    fn the_written_toml_reads_back_as_the_same_profile() {
        let directory = tempfile::tempdir().unwrap();
        let value = json!({"install":"pnpm install --frozen-lockfile","start":"pnpm dev","env":{"APP_ENV":"test"},"ports":[5432],"window":{"width":390,"height":844}});
        let written = write(directory.path(), value.clone()).unwrap();
        let read = Profile::read("demo", &written.path).unwrap();
        assert_eq!(read, serde_json::from_value::<Profile>(value).unwrap());
        assert_eq!(
            std::fs::read_to_string(&written.path).unwrap(),
            written.toml
        );
    }
}
