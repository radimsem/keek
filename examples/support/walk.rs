use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Every directory under `root` whose name ends with `suffix`, sorted. It does not descend into a match.
pub fn dirs_ending(root: &Path, suffix: &str) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    walk(root, suffix, true, &mut found)?;
    found.sort();
    Ok(found)
}

/// Every file under `root` whose name ends with `suffix`, sorted.
pub fn files_ending(root: &Path, suffix: &str) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    walk(root, suffix, false, &mut found)?;
    found.sort();
    Ok(found)
}

fn walk(dir: &Path, suffix: &str, want_dirs: bool, found: &mut Vec<PathBuf>) -> Result<()> {
    let entries = fs::read_dir(dir).with_context(|| {
        format!(
            "cannot read {}. Check the evidence root path.",
            dir.display()
        )
    })?;
    for entry in entries {
        let path = entry?.path();
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        if path.is_dir() {
            if want_dirs && name.ends_with(suffix) {
                found.push(path);
            } else {
                walk(&path, suffix, want_dirs, found)?;
            }
        } else if !want_dirs && name.ends_with(suffix) {
            found.push(path);
        }
    }
    Ok(())
}
