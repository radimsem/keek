use crate::credentials::{CredentialsError, check_length, entries, public_name, valid_name};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Which named Test Credentials keek wrote, never their values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CopiedCredentials {
    pub path: PathBuf,
    pub copied: Vec<String>,
    /// Not in the source, so the value already in the file stays.
    pub kept: Vec<String>,
    /// Not in the source and not in the file, so the developer fills them in.
    pub missing: Vec<String>,
    pub public: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum CopyError {
    #[error(
        "The credentials source {0} is not an absolute path. Pass the full host path of the file to copy from."
    )]
    RelativeSource(PathBuf),
    #[error(
        "{0:?} is not a valid Test Credential name. Use letters, digits and _, not starting with a digit."
    )]
    InvalidName(String),
    #[error(
        "Public Test Credential {0} is not in names. Add it to names or remove it from public."
    )]
    UnrequestedPublic(String),
    #[error("Cannot read the credentials source {path}: {source}. Check the path and try again.", path = .path.display())]
    ReadSource {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("Cannot read the Test Credentials file {path}: {source}. Check its permissions and try again.", path = .path.display())]
    ReadTarget {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error(transparent)]
    Parse(#[from] CredentialsError),
    #[error("Cannot write Test Credentials to {path}: {source}. Check the folder's permissions and try again.", path = .path.display())]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
}

/// Copies `names` and replaces their public markers, keeping other names and their markers.
pub fn copy_credentials(
    target: &Path,
    source: &Path,
    names: &[String],
    public: &[String],
) -> Result<CopiedCredentials, CopyError> {
    if !source.is_absolute() {
        return Err(CopyError::RelativeSource(source.to_owned()));
    }
    let mut requested: Vec<&str> = Vec::new();
    for name in names {
        if !valid_name(name) {
            return Err(CopyError::InvalidName(name.clone()));
        }
        if !requested.contains(&name.as_str()) {
            requested.push(name);
        }
    }
    for name in public {
        if !requested.contains(&name.as_str()) {
            return Err(CopyError::UnrequestedPublic(name.clone()));
        }
    }
    let text = std::fs::read_to_string(source).map_err(|error| CopyError::ReadSource {
        path: source.to_owned(),
        source: error,
    })?;
    let mut values: BTreeMap<&str, &str> = BTreeMap::new();
    for (index, line) in text.lines().enumerate() {
        // A repository's .env can hold lines keek's format doesn't, such as a multi-line value. Only one that seems to define a name asked for refuses.
        let Ok(found) = entries(source, line) else {
            let line = line.trim_start();
            let line = line.strip_prefix("export ").unwrap_or(line);
            if let Some((name, _)) = line.split_once('=')
                && requested.contains(&name.trim())
            {
                return Err(CredentialsError::Malformed {
                    path: source.to_owned(),
                    line: index + 1,
                }
                .into());
            }
            continue;
        };
        for entry in found {
            if !requested.contains(&entry.name) {
                continue;
            }
            if opens_multi_line_value(line) {
                return Err(CredentialsError::Malformed {
                    path: source.to_owned(),
                    line: index + 1,
                }
                .into());
            }
            if values.insert(entry.name, entry.value).is_some() {
                return Err(CredentialsError::Duplicate {
                    path: source.to_owned(),
                    line: index + 1,
                    name: entry.name.into(),
                }
                .into());
            }
            check_length(source, entry.name, entry.value)?;
        }
    }
    let existing = match std::fs::read_to_string(target) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(CopyError::ReadTarget {
                path: target.to_owned(),
                source: error,
            });
        }
    };
    let held: Vec<&str> = entries(target, &existing)?
        .into_iter()
        .map(|entry| entry.name)
        .collect();
    let mut copied = CopiedCredentials {
        path: target.to_owned(),
        copied: vec![],
        kept: vec![],
        missing: vec![],
        public: requested
            .iter()
            .filter(|name| public.iter().any(|public| public == **name))
            .map(|name| (*name).to_owned())
            .collect(),
    };
    for name in &requested {
        let list = if values.contains_key(name) {
            &mut copied.copied
        } else if held.contains(name) {
            &mut copied.kept
        } else {
            &mut copied.missing
        };
        list.push((*name).to_owned());
    }
    if requested.is_empty() {
        return Ok(copied);
    }
    let mut lines = Vec::new();
    let mut written = Vec::new();
    for line in existing.lines() {
        if public_name(line).is_some_and(|name| requested.contains(&name)) {
            continue;
        }
        let defined = entries(target, line)?.first().map(|entry| entry.name);
        match defined.and_then(|name| values.get_key_value(name)) {
            Some((name, value)) => {
                lines.push(line_for(name, value));
                written.push(*name);
            }
            None => lines.push(line.to_owned()),
        }
    }
    for (name, value) in &values {
        if !written.contains(name) {
            lines.push(line_for(name, value));
        }
    }
    for name in &copied.public {
        lines.push(format!("# keek: public {name}"));
    }
    if lines.is_empty() && existing.is_empty() {
        return Ok(copied);
    }
    let mut text = lines.join("\n");
    if !lines.is_empty() {
        text.push('\n');
    }
    write_private(target, &text).map_err(|error| CopyError::Write {
        path: target.to_owned(),
        source: error,
    })?;
    Ok(copied)
}

// A quote with no matching quote at the line's end opens a value that continues on the next lines.
fn opens_multi_line_value(line: &str) -> bool {
    let Some((_, value)) = line.split_once('=') else {
        return false;
    };
    let value = value.strip_suffix('\r').unwrap_or(value);
    value.chars().next().is_some_and(|quote| {
        matches!(quote, '"' | '\'') && !(value.len() >= 2 && value.ends_with(quote))
    })
}

// Outer single quotes are stripped literally, so any one-line value reads back unchanged.
fn line_for(name: &str, value: &str) -> String {
    format!("{name}='{value}'")
}

fn write_private(target: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
    if let Some(directory) = target.parent() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(directory)?;
    }
    let temporary = target.with_extension("env.tmp");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temporary)?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    file.write_all(text.as_bytes())?;
    std::fs::rename(temporary, target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn named_credentials_are_copied_with_mode_600_and_the_reply_holds_only_names() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join(".env");
        std::fs::write(
            &source,
            "PORT=3000\nexport TEST_PASSWORD=\"made-up secret\"\nTEST_USER=throwaway-user\n",
        )
        .unwrap();
        let target = directory.path().join("demo.env");
        let copied = copy_credentials(
            &target,
            &source,
            &names(&["TEST_PASSWORD", "TEST_USER"]),
            &[],
        )
        .unwrap();
        assert_eq!(copied.copied, ["TEST_PASSWORD", "TEST_USER"]);
        assert!(copied.kept.is_empty() && copied.missing.is_empty());
        let reply = serde_json::to_string(&copied).unwrap();
        assert!(!reply.contains("made-up secret") && !reply.contains("throwaway-user"));
        let mode = std::fs::metadata(&target).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let (credentials, warning) = crate::credentials::Credentials::read(&target).unwrap();
        assert!(warning.is_none());
        assert_eq!(credentials.names(), ["TEST_PASSWORD", "TEST_USER"]);
        assert_eq!(credentials.mask("made-up secret"), "[masked]");
        assert!(!std::fs::read_to_string(&target).unwrap().contains("PORT"));
    }

    #[test]
    fn copy_keeps_other_names_and_existing_values_for_names_the_source_lacks() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join(".env");
        std::fs::write(&source, "TEST_PASSWORD=new-made-up\n").unwrap();
        let target = directory.path().join("demo.env");
        std::fs::write(
            &target,
            "# filled in by hand\nOTHER=other-made-up\nTEST_PASSWORD=old-made-up\nTEST_TOKEN=kept-made-up\n",
        )
        .unwrap();
        let copied = copy_credentials(
            &target,
            &source,
            &names(&["TEST_PASSWORD", "TEST_TOKEN", "TEST_CODE"]),
            &[],
        )
        .unwrap();
        assert_eq!(copied.copied, ["TEST_PASSWORD"]);
        assert_eq!(copied.kept, ["TEST_TOKEN"]);
        assert_eq!(copied.missing, ["TEST_CODE"]);
        let text = std::fs::read_to_string(&target).unwrap();
        assert!(
            text.starts_with("# filled in by hand\nOTHER=other-made-up\n"),
            "{text}"
        );
        assert!(
            text.contains("new-made-up") && !text.contains("old-made-up"),
            "{text}"
        );
        assert!(text.contains("TEST_TOKEN=kept-made-up"), "{text}");
    }

    #[test]
    fn copy_refuses_a_relative_source_and_a_short_value_without_echoing_it() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("demo.env");
        let error = copy_credentials(&target, Path::new(".env"), &names(&["A"]), &[])
            .unwrap_err()
            .to_string();
        assert!(error.contains("absolute"), "{error}");
        let source = directory.path().join(".env");
        std::fs::write(&source, "PIN=1234\n").unwrap();
        let error = copy_credentials(&target, &source, &names(&["PIN"]), &[])
            .unwrap_err()
            .to_string();
        assert!(error.contains("PIN") && !error.contains("1234"), "{error}");
        assert!(!target.exists());
    }

    #[test]
    fn a_source_line_keek_cannot_read_does_not_block_the_names_asked_for() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join(".env");
        std::fs::write(
            &source,
            "KEY = spaced\nPEM=\"-----BEGIN\nMIIBmadeup\n-----END\"\nTEST_PASSWORD=made-up-secret\n",
        )
        .unwrap();
        let target = directory.path().join("demo.env");
        let copied = copy_credentials(&target, &source, &names(&["TEST_PASSWORD"]), &[]).unwrap();
        assert_eq!(copied.copied, ["TEST_PASSWORD"]);
    }

    #[test]
    fn a_requested_name_on_a_line_keek_cannot_read_refuses_with_the_line() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join(".env");
        std::fs::write(&source, "PORT=3000\nTEST_PASSWORD = made-up-secret\n").unwrap();
        let target = directory.path().join("demo.env");
        let error = copy_credentials(&target, &source, &names(&["TEST_PASSWORD"]), &[])
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("line 2") && !error.contains("made-up-secret"),
            "{error}"
        );
        assert!(!target.exists());
    }

    #[test]
    fn a_requested_name_opening_a_multi_line_value_refuses_and_keeps_the_old_value() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join(".env");
        std::fs::write(
            &source,
            "PORT=3000\nTEST_KEY=\"-----BEGIN MADE UP KEY-----\nMIIBmadeup\n-----END MADE UP KEY-----\"\n",
        )
        .unwrap();
        let target = directory.path().join("demo.env");
        std::fs::write(&target, "TEST_KEY='working-made-up'\n").unwrap();
        let error = copy_credentials(&target, &source, &names(&["TEST_KEY"]), &[])
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("line 2") && !error.contains("BEGIN"),
            "{error}"
        );
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "TEST_KEY='working-made-up'\n"
        );
        std::fs::write(&source, "TEST_QUOTED='\"made-up-quoted\"'\n").unwrap();
        let copied = copy_credentials(&target, &source, &names(&["TEST_QUOTED"]), &[]).unwrap();
        assert_eq!(copied.copied, ["TEST_QUOTED"]);
    }

    #[test]
    fn copying_marks_public_names_and_keeps_markers_for_other_names() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.env");
        let target = directory.path().join("project.env");
        std::fs::write(&source, "USER=operator\nPASSWORD=made-up-password\n").unwrap();
        std::fs::write(&target, "# keek: public OTHER\nOTHER=other-user\n").unwrap();
        let copied = copy_credentials(
            &target,
            &source,
            &names(&["USER", "PASSWORD", "USER"]),
            &names(&["USER", "USER"]),
        )
        .unwrap();
        let text = std::fs::read_to_string(&target).unwrap();
        assert_eq!(
            text.lines()
                .filter(|line| *line == "# keek: public USER")
                .count(),
            1
        );
        assert!(text.starts_with("# keek: public OTHER\nOTHER=other-user\n"));
        assert!(!text.contains("# keek: public PASSWORD"));
        assert_eq!(copied.public, ["USER"]);
        let (credentials, _) = crate::credentials::Credentials::read(&target).unwrap();
        assert_eq!(
            credentials.mask("operator other-user made-up-password"),
            "operator other-user [masked]"
        );
    }

    #[test]
    fn copying_removes_a_requested_secret_names_marker_even_if_the_source_lacks_it() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.env");
        let target = directory.path().join("project.env");
        std::fs::write(&source, "").unwrap();
        std::fs::write(&target, "# keek: public USER\nUSER=operator\n").unwrap();
        let copied = copy_credentials(&target, &source, &names(&["USER"]), &[]).unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "USER=operator\n");
        assert_eq!(copied.kept, ["USER"]);
        assert!(copied.public.is_empty());
        let (credentials, _) = crate::credentials::Credentials::read(&target).unwrap();
        assert_eq!(credentials.mask("operator"), "[masked]");
    }

    #[test]
    fn copying_marks_a_public_name_even_if_its_value_is_kept_or_missing() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.env");
        let target = directory.path().join("project.env");
        std::fs::write(&source, "").unwrap();
        std::fs::write(&target, "USER=operator\n").unwrap();
        let copied = copy_credentials(
            &target,
            &source,
            &names(&["USER", "MISSING"]),
            &names(&["USER", "MISSING"]),
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "USER=operator\n# keek: public USER\n# keek: public MISSING\n"
        );
        assert_eq!(copied.public, ["USER", "MISSING"]);
        assert_eq!(copied.kept, ["USER"]);
        assert_eq!(copied.missing, ["MISSING"]);
    }

    #[test]
    fn copying_refuses_a_public_name_that_was_not_requested() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.env");
        let target = directory.path().join("project.env");
        std::fs::write(&source, "PASSWORD=made-up-password\n").unwrap();
        let error = copy_credentials(&target, &source, &names(&["PASSWORD"]), &names(&["USER"]))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("USER") && error.contains("Add it to names or remove it from public"),
            "{error}"
        );
        assert!(!target.exists());
    }

    #[test]
    fn values_with_quotes_and_spaces_read_back_unchanged() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join(".env");
        std::fs::write(&source, "A=' padded \"quoted\" '\n").unwrap();
        let target = directory.path().join("demo.env");
        copy_credentials(&target, &source, &names(&["A"]), &[]).unwrap();
        let (credentials, _) = crate::credentials::Credentials::read(&target).unwrap();
        let mut value = serde_json::json!("${A}");
        credentials.substitute(&mut value).unwrap();
        assert_eq!(value, " padded \"quoted\" ");
    }
}
