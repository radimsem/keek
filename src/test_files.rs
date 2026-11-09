use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const MAX_LISTED: usize = 100;
pub fn folder(projects_config: &Path, project: &str) -> PathBuf {
    projects_config.join(format!("{project}.files"))
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestFileList {
    pub names: Vec<String>,
    pub more: usize,
}
#[derive(Debug, Clone, Default)]
pub struct TestFiles {
    pub folder: Option<PathBuf>,
    pub list: TestFileList,
}
#[derive(Debug, thiserror::Error)]
pub enum TestFilesError {
    #[error("{} is not a folder, so keek can't offer its Test Files. Remove it or replace it with a folder, then open again.", .0.display())]
    NotAFolder(PathBuf),
    #[error("{} is a symlink, and keek binds only a real folder as Test Files. Replace it with a folder holding copies of the files, then open again.", .0.display())]
    Symlink(PathBuf),
    #[error(
        "Cannot read Test Files at {path}: {source}. Check the folder and its permissions, then try again."
    )]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}
impl TestFiles {
    pub async fn read(projects_config: &Path, project: &str) -> Result<Self, TestFilesError> {
        let folder = folder(projects_config, project);
        let path = folder.clone();
        tokio::task::spawn_blocking(move || {
            let metadata = match std::fs::symlink_metadata(&folder) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(Self::default());
                }
                Err(source) => {
                    return Err(TestFilesError::Io {
                        path: folder,
                        source,
                    });
                }
            };
            if metadata.is_symlink() {
                return Err(TestFilesError::Symlink(folder));
            }
            if !metadata.is_dir() {
                return Err(TestFilesError::NotAFolder(folder));
            }
            let mut names = Vec::new();
            walk(&folder, Path::new(""), &mut names)?;
            names.sort();
            let more = names.len().saturating_sub(MAX_LISTED);
            names.truncate(MAX_LISTED);
            Ok(Self {
                folder: Some(folder),
                list: TestFileList { names, more },
            })
        })
        .await
        .map_err(|error| TestFilesError::Io {
            path,
            source: std::io::Error::other(error),
        })?
    }
}
fn walk(root: &Path, relative: &Path, names: &mut Vec<String>) -> Result<(), TestFilesError> {
    let directory = root.join(relative);
    let io = |source| TestFilesError::Io {
        path: directory.clone(),
        source,
    };
    for entry in std::fs::read_dir(&directory).map_err(io)? {
        let entry = entry.map_err(io)?;
        let kind = entry.file_type().map_err(io)?;
        let path = relative.join(entry.file_name());
        // A symlink's host target usually doesn't exist in the sandbox, so listing it would name a dangling path.
        if kind.is_dir() {
            walk(root, &path, names)?;
        } else if kind.is_file() {
            names.push(path.to_string_lossy().into_owned());
        }
    }
    Ok(())
}

pub const SANDBOX_FILES: &str = "/keek/files";
pub(crate) fn bind_args(folder: &Path) -> [std::ffi::OsString; 3] {
    ["--ro-bind".into(), folder.into(), SANDBOX_FILES.into()]
}

#[cfg(test)]
#[path = "test_files_tests.rs"]
mod tests;
