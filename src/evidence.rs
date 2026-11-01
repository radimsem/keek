use crate::process::ProcessRunner;
use crate::report::{self, ReportInputs};
use chrono::{DateTime, Local};
use serde::Serialize;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum EvidenceError {
    #[error(
        "Cannot use the videos folder: {0}. Configure an absolute VIDEOS folder keek can write to with xdg-user-dirs, then call close_session again."
    )]
    Videos(String),
    #[error(
        "Cannot file Evidence from {folder}: {source}. The session folder stays in place. Check that folder, the videos folder's permissions and available space."
    )]
    Filing { folder: PathBuf, source: io::Error },
}
#[derive(Debug, Serialize)]
pub struct Filed {
    pub report: PathBuf,
    pub raw: Option<PathBuf>,
    pub timeline: PathBuf,
    pub actions: PathBuf,
    pub verification: Option<PathBuf>,
    pub index: PathBuf,
}
pub async fn videos_dir(runner: &dyn ProcessRunner) -> Result<PathBuf, EvidenceError> {
    let output = runner
        .output("xdg-user-dir", &["VIDEOS"], None)
        .await
        .map_err(|e| EvidenceError::Videos(e.to_string()))?;
    let path = PathBuf::from(output.trim());
    if !path.is_absolute() {
        return Err(EvidenceError::Videos(
            "xdg-user-dir returned an empty or relative path".into(),
        ));
    }
    Ok(path)
}
pub async fn writable_videos_dir(runner: &dyn ProcessRunner) -> Result<PathBuf, EvidenceError> {
    let videos = videos_dir(runner).await?;
    let keek = videos.join("keek");
    let created = keek.clone();
    tokio::task::spawn_blocking(move || fs::create_dir_all(created))
        .await
        .map_err(|e| EvidenceError::Videos(e.to_string()))?
        .map_err(|e| EvidenceError::Videos(format!("{}: {e}", keek.display())))?;
    Ok(videos)
}
pub fn append_index(path: &Path, line: &str) -> io::Result<()> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.lock()?;
    file.write_all(line.as_bytes())?;
    file.sync_all()
}
fn open_source(path: &Path) -> io::Result<Option<File>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    if !metadata.is_file() {
        tracing::warn!(?path, "skipping non-regular Evidence source");
        return Ok(None);
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    if !file.metadata()?.is_file() {
        tracing::warn!(?path, "skipping non-regular Evidence source");
        return Ok(None);
    }
    Ok(Some(file))
}
fn copy_file(source: &Path, dest: &Path) -> io::Result<bool> {
    let Some(mut source) = open_source(source)? else {
        return Ok(false);
    };
    let mut dest = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(dest)?;
    io::copy(&mut source, &mut dest)?;
    dest.sync_all()?;
    source.rewind()?;
    dest.rewind()?;
    let mut left = [0; 64 * 1024];
    let mut right = [0; 64 * 1024];
    loop {
        let n = source.read(&mut left)?;
        dest.read_exact(&mut right[..n])?;
        if left[..n] != right[..n] {
            return Err(io::Error::other("Evidence copy differs from its source"));
        }
        if n == 0 {
            if dest.read(&mut right[..1])? != 0 {
                return Err(io::Error::other("Evidence copy has extra bytes"));
            }
            break;
        }
    }
    Ok(true)
}
fn copy_directory(source: &Path, dest: &Path) -> io::Result<()> {
    fs::create_dir(dest)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        if metadata.is_dir() {
            copy_directory(&entry.path(), &dest.join(entry.file_name()))?;
        } else {
            copy_file(&entry.path(), &dest.join(entry.file_name()))?;
        }
    }
    Ok(())
}
pub fn file(
    folder: &Path,
    videos: &Path,
    project: &str,
    session: &str,
    started: DateTime<Local>,
    input: &ReportInputs<'_>,
) -> Result<Filed, EvidenceError> {
    let result = (|| -> io::Result<Filed> {
        let project_dir = videos.join("keek").join(project);
        let date = started.format("%Y-%m-%d").to_string();
        let date_dir = project_dir.join(&date);
        fs::create_dir_all(&date_dir)?;
        let base = format!("{project}_{}", started.format("%H-%M-%S"));
        let mut stem = session.to_owned();
        let mut suffix = session
            .strip_prefix(&format!("{base}-"))
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(1)
            + 1;
        loop {
            let prefix = format!("{stem}_");
            let mut occupied = false;
            for entry in fs::read_dir(&date_dir)? {
                if entry?.file_name().to_string_lossy().starts_with(&prefix) {
                    occupied = true;
                    break;
                }
            }
            if !occupied {
                break;
            }
            stem = format!("{base}-{suffix}");
            suffix += 1;
        }
        let prefix = format!("{stem}_{}", input.end.filename_word());
        let path = |kind: &str| date_dir.join(format!("{prefix}_{kind}"));
        let actions = path("actions");
        fs::create_dir(&actions)?;
        for entry in fs::read_dir(folder)? {
            let entry = entry?;
            if entry.file_name().to_string_lossy().starts_with("turn-") {
                if fs::symlink_metadata(entry.path())?.is_dir() {
                    copy_directory(&entry.path(), &actions.join(entry.file_name()))?;
                } else {
                    tracing::warn!(path=?entry.path(), "skipping non-directory Action Log");
                }
            }
        }
        let timeline = path("timeline.json");
        if !copy_file(&folder.join("timeline.json"), &timeline)? {
            return Err(io::Error::other(
                "required timeline.json is missing or not regular",
            ));
        }
        let optional = |source: &str, kind: &str| -> io::Result<Option<PathBuf>> {
            let dest = path(kind);
            Ok(copy_file(&folder.join(source), &dest)?.then_some(dest))
        };
        let raw = optional("recording.mp4", "raw.mp4")?;
        let verification = optional("verification.mp4", "verification.mp4")?;
        let report = path("report.md");
        let mut report_file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&report)?;
        report_file.write_all(report::render(input).as_bytes())?;
        report_file.sync_all()?;
        let index = project_dir.join("index.md");
        let filed = Filed {
            report,
            raw,
            timeline,
            actions,
            verification,
            index,
        };
        let mut links = Vec::new();
        for (label, p) in [
            ("report", Some(&filed.report)),
            ("raw", filed.raw.as_ref()),
            ("timeline", Some(&filed.timeline)),
            ("actions", Some(&filed.actions)),
            ("verification", filed.verification.as_ref()),
        ] {
            if let Some(p) = p {
                links.push((
                    label,
                    format!(
                        "{date}/{}",
                        p.file_name().unwrap_or_default().to_string_lossy()
                    ),
                ));
            }
        }
        append_index(
            &filed.index,
            &report::index_line(
                &date,
                &started.format("%H:%M:%S").to_string(),
                input.revision,
                input.end,
                input.goal,
                &links,
            ),
        )?;
        Ok(filed)
    })();
    let filed = result.map_err(|source| EvidenceError::Filing {
        folder: folder.to_owned(),
        source,
    })?;
    if let Err(error) = fs::remove_dir_all(folder) {
        tracing::error!(?folder, %error, "Evidence filed but session folder could not be removed");
    }
    Ok(filed)
}
