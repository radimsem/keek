use std::{fs, io, path::Path};

pub(crate) const BAR_OFF: &str = "bar-off";

pub(crate) fn prepare_session_toggles(host: &Path, session: &Path) -> io::Result<()> {
    match copy_directory(host, session) {
        Err(error)
            if !matches!(
                error.kind(),
                io::ErrorKind::NotFound
                    | io::ErrorKind::NotADirectory
                    | io::ErrorKind::PermissionDenied
            ) =>
        {
            return Err(error);
        }
        Ok(()) | Err(_) => {}
    }
    let flag = session.join(BAR_OFF);
    // A copy of a read-only host flag can't be written over.
    match fs::remove_file(&flag) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
        Ok(()) | Err(_) => {}
    }
    // The Omarchy shell reads bar-off at start, so the flag must exist before launch.
    fs::write(flag, b"")
}

fn copy_directory(host: &Path, session: &Path) -> io::Result<()> {
    fs::create_dir_all(session)?;
    for entry in fs::read_dir(host)? {
        match entry.and_then(|entry| copy_entry(&entry, session)) {
            Err(error) if error.kind() != io::ErrorKind::PermissionDenied => return Err(error),
            Ok(()) | Err(_) => {}
        }
    }
    Ok(())
}

fn copy_entry(entry: &fs::DirEntry, session: &Path) -> io::Result<()> {
    let kind = entry.file_type()?;
    let destination = session.join(entry.file_name());
    if kind.is_dir() {
        copy_directory(&entry.path(), &destination)
    } else if kind.is_file() {
        fs::copy(entry.path(), destination).map(drop)
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[path = "omarchy_toggles_tests.rs"]
mod tests;
