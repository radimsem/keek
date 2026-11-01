// Stopgap removed by #12 when Cua Driver masks the Action Log.
use crate::credentials::Credentials;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
#[derive(Debug, thiserror::Error)]
pub enum ScrubError {
    #[error("Cannot remove Test Credentials from {path}: {source}.")]
    Io { path: PathBuf, source: io::Error },
    #[error("Cannot remove Test Credentials from {0}: the file exceeds 64 MiB.")]
    TooLarge(PathBuf),
}
const LIMIT: u64 = 64 * 1024 * 1024;
pub fn scrub_folder(credentials: &Credentials, folder: &Path) -> Result<(), ScrubError> {
    if credentials.names().is_empty() {
        return Ok(());
    }
    let error = |source| ScrubError::Io {
        path: folder.into(),
        source,
    };
    for entry in fs::read_dir(folder).map_err(error)? {
        let entry = entry.map_err(error)?;
        if entry.file_name().to_string_lossy().starts_with("turn-")
            && fs::symlink_metadata(entry.path()).map_err(error)?.is_dir()
        {
            visit(credentials, &entry.path())?;
        }
    }
    let timeline = folder.join("timeline.json");
    match fs::symlink_metadata(&timeline) {
        Ok(_) => visit(credentials, &timeline)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(ScrubError::Io {
                path: timeline,
                source,
            });
        }
    }
    Ok(())
}
fn visit(credentials: &Credentials, path: &Path) -> Result<(), ScrubError> {
    let error = |source| ScrubError::Io {
        path: path.into(),
        source,
    };
    let metadata = fs::symlink_metadata(path).map_err(error)?;
    if metadata.is_dir() {
        for entry in fs::read_dir(path).map_err(error)? {
            visit(credentials, &entry.map_err(error)?.path())?;
        }
    } else if metadata.is_file() {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
            .map_err(error)?;
        let metadata = file.metadata().map_err(error)?;
        if !metadata.is_file() {
            return Ok(());
        }
        if metadata.len() > LIMIT {
            return Err(ScrubError::TooLarge(path.into()));
        }
        let mut bytes = Vec::new();
        file.take(LIMIT + 1)
            .read_to_end(&mut bytes)
            .map_err(error)?;
        if bytes.len() as u64 > LIMIT {
            return Err(ScrubError::TooLarge(path.into()));
        }
        if let Ok(text) = String::from_utf8(bytes) {
            let masked = credentials.mask(&text);
            if text != masked {
                let temporary = path.with_file_name(format!(".keek-mask-{}", std::process::id()));
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&temporary)
                    .map_err(error)?;
                let result = (|| {
                    file.write_all(masked.as_bytes())?;
                    file.sync_all()?;
                    fs::rename(&temporary, path)
                })()
                .map_err(error);
                if result.is_err() {
                    let _ = fs::remove_file(&temporary);
                }
                result?;
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::Credentials;
    fn credentials() -> Credentials {
        Credentials::parse(
            std::path::Path::new("fixture.env"),
            "PASSWORD=pa\"ss\\w&rd 1",
        )
        .unwrap()
    }
    #[test]
    fn text_evidence_is_masked_and_binary_bytes_stay_unchanged() {
        let root = tempfile::tempdir().unwrap();
        let turn = root.path().join("turn-00001");
        std::fs::create_dir(&turn).unwrap();
        for name in ["action.json", "events.jsonl", "notes.txt"] {
            std::fs::write(
                turn.join(name),
                "pa\"ss\\w&rd 1 pa\\\"ss\\\\w&rd 1 pa%22ss%5Cw%26rd%201",
            )
            .unwrap();
        }
        std::fs::write(root.path().join("timeline.json"), "pa\"ss\\w&rd 1").unwrap();
        std::fs::write(turn.join("image.png"), [0xff, 0, 1]).unwrap();
        scrub_folder(&credentials(), root.path()).unwrap();
        for name in ["action.json", "events.jsonl", "notes.txt"] {
            assert_eq!(
                std::fs::read_to_string(turn.join(name)).unwrap(),
                "[masked] [masked] [masked]"
            );
        }
        assert_eq!(
            std::fs::read_to_string(root.path().join("timeline.json")).unwrap(),
            "[masked]"
        );
        assert_eq!(std::fs::read(turn.join("image.png")).unwrap(), [0xff, 0, 1]);
    }
    #[test]
    fn symlinks_leave_outside_files_unchanged() {
        let root = tempfile::tempdir().unwrap();
        let outside = root.path().join("outside");
        std::fs::write(&outside, "pa\"ss\\w&rd 1").unwrap();
        let turn = root.path().join("turn-00001");
        std::fs::create_dir(&turn).unwrap();
        std::os::unix::fs::symlink(&outside, turn.join("link")).unwrap();
        scrub_folder(&credentials(), root.path()).unwrap();
        assert_eq!(std::fs::read_to_string(outside).unwrap(), "pa\"ss\\w&rd 1");
    }
    #[test]
    fn oversized_files_refuse_filing() {
        let root = tempfile::tempdir().unwrap();
        let turn = root.path().join("turn-00001");
        std::fs::create_dir(&turn).unwrap();
        let path = turn.join("large");
        std::fs::File::create(&path)
            .unwrap()
            .set_len(64 * 1024 * 1024 + 1)
            .unwrap();
        assert!(
            scrub_folder(&credentials(), root.path())
                .unwrap_err()
                .to_string()
                .contains(path.to_str().unwrap())
        );
    }
    #[test]
    fn no_credentials_touches_nothing() {
        let root = tempfile::tempdir().unwrap();
        let absent = root.path().join("absent");
        scrub_folder(&Credentials::default(), &absent).unwrap();
        assert!(!absent.exists());
    }
}
