use crate::credentials::{CredentialsError, check_length, entries, valid_name};
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
    #[error("Cannot read the credentials source {path}: {source}. Check the path and try again.", path = .path.display())]
    ReadSource {
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

/// Copies `names` from `source` into `target`, keeping every other line of `target`.
pub fn copy_credentials(
    target: &Path,
    source: &Path,
    names: &[String],
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
    let text = std::fs::read_to_string(source).map_err(|error| CopyError::ReadSource {
        path: source.to_owned(),
        source: error,
    })?;
    let mut values: BTreeMap<&str, &str> = BTreeMap::new();
    for entry in entries(source, &text)? {
        if !requested.contains(&entry.name) {
            continue;
        }
        if values.insert(entry.name, entry.value).is_some() {
            return Err(CredentialsError::Duplicate {
                path: source.to_owned(),
                line: entry.line,
                name: entry.name.into(),
            }
            .into());
        }
        check_length(source, entry.name, entry.value)?;
    }
    let existing = match std::fs::read_to_string(target) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(CopyError::ReadSource {
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
    if values.is_empty() {
        return Ok(copied);
    }
    let mut lines = Vec::new();
    let mut written = Vec::new();
    for line in existing.lines() {
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
    write_private(target, &(lines.join("\n") + "\n")).map_err(|error| CopyError::Write {
        path: target.to_owned(),
        source: error,
    })?;
    Ok(copied)
}

// Outer single quotes are stripped literally, so any one-line value reads back unchanged.
fn line_for(name: &str, value: &str) -> String {
    format!("{name}='{value}'")
}

fn write_private(target: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
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
        let copied =
            copy_credentials(&target, &source, &names(&["TEST_PASSWORD", "TEST_USER"])).unwrap();
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
        let error = copy_credentials(&target, Path::new(".env"), &names(&["A"]))
            .unwrap_err()
            .to_string();
        assert!(error.contains("absolute"), "{error}");
        let source = directory.path().join(".env");
        std::fs::write(&source, "PIN=1234\n").unwrap();
        let error = copy_credentials(&target, &source, &names(&["PIN"]))
            .unwrap_err()
            .to_string();
        assert!(error.contains("PIN") && !error.contains("1234"), "{error}");
        assert!(!target.exists());
    }

    #[test]
    fn values_with_quotes_and_spaces_read_back_unchanged() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join(".env");
        std::fs::write(&source, "A=' padded \"quoted\" '\n").unwrap();
        let target = directory.path().join("demo.env");
        copy_credentials(&target, &source, &names(&["A"])).unwrap();
        let (credentials, _) = crate::credentials::Credentials::read(&target).unwrap();
        let mut value = serde_json::json!("${A}");
        credentials.substitute(&mut value).unwrap();
        assert_eq!(value, " padded \"quoted\" ");
    }
}
