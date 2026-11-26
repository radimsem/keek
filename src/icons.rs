use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum IconError {
    #[error(
        "Could not install the keek icon at {}: {source}. Notifications still work without it. Check that the folder is writable.",
        path.display()
    )]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
}

const ICONS: [(&str, &[u8]); 3] = [
    ("48x48", include_bytes!("../assets/icons/keek-48.png")),
    ("64x64", include_bytes!("../assets/icons/keek-64.png")),
    ("128x128", include_bytes!("../assets/icons/keek-128.png")),
];

/// A file that already holds the same bytes is left alone, so its modification time stays put.
pub fn install(theme_dir: &Path) -> Result<(), IconError> {
    for (size, bytes) in ICONS {
        let path = theme_dir.join(size).join("apps/keek.png");
        if std::fs::read(&path).is_ok_and(|current| current == bytes) {
            continue;
        }
        write(&path, bytes).map_err(|source| IconError::Write { path, source })?;
    }
    Ok(())
}

fn write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)?;
    }
    std::fs::write(path, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    const SIZES: [&str; 3] = ["48x48", "64x64", "128x128"];

    fn icon(theme: &Path, size: &str) -> PathBuf {
        theme.join(size).join("apps/keek.png")
    }

    #[test]
    fn install_writes_each_size_once() {
        let root = tempfile::tempdir().unwrap();

        install(root.path()).unwrap();

        for size in SIZES {
            let bytes = std::fs::read(icon(root.path(), size)).unwrap();
            assert!(bytes.starts_with(b"\x89PNG"), "{size} is not a PNG");
        }
        assert_ne!(
            std::fs::read(icon(root.path(), "48x48")).unwrap(),
            std::fs::read(icon(root.path(), "128x128")).unwrap()
        );
    }

    #[test]
    fn install_rewrites_an_icon_whose_bytes_differ() {
        let root = tempfile::tempdir().unwrap();
        install(root.path()).unwrap();
        let expected = std::fs::read(icon(root.path(), "64x64")).unwrap();
        std::fs::write(icon(root.path(), "64x64"), b"old logo").unwrap();

        install(root.path()).unwrap();

        assert_eq!(std::fs::read(icon(root.path(), "64x64")).unwrap(), expected);
    }

    #[test]
    fn install_leaves_a_matching_icon_untouched() {
        let root = tempfile::tempdir().unwrap();
        install(root.path()).unwrap();
        let path = icon(root.path(), "48x48");
        let long_ago = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(long_ago)
            .unwrap();

        install(root.path()).unwrap();

        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(modified, long_ago);
    }

    #[test]
    fn install_names_the_path_it_could_not_write() {
        let root = tempfile::tempdir().unwrap();
        let blocked = root.path().join("file");
        std::fs::write(&blocked, b"not a folder").unwrap();

        let error = install(&blocked).unwrap_err();

        assert!(error.to_string().contains("keek.png"));
    }
}
