#[derive(Debug, PartialEq, Eq)]
enum Restore {
    Clear,
    Text(String),
    Single(String),
    Refuse,
}

fn restorable(types: &[&str]) -> Restore {
    let payload: Vec<_> = types
        .iter()
        .copied()
        .filter(|t| *t != "x-kde-passwordManagerHint")
        .collect();
    if types.is_empty() {
        return Restore::Clear;
    }
    let text = |t: &str| {
        matches!(
            t,
            "text/plain"
                | "text/plain;charset=utf-8"
                | "text/plain;charset=UTF-8"
                | "UTF8_STRING"
                | "TEXT"
                | "STRING"
        )
    };
    if !payload.is_empty() && payload.iter().all(|t| text(t)) {
        let mime = payload
            .iter()
            .find(|t| t.starts_with("text/plain"))
            .copied()
            .unwrap_or(payload[0]);
        return Restore::Text(mime.into());
    }
    match payload.as_slice() {
        [mime] => Restore::Single((*mime).into()),
        [] | [_, _, ..] => Restore::Refuse,
    }
}

struct SavedClipboard {
    directory: TempDir,
    restore: Restore,
    sensitive: bool,
    bytes: Vec<u8>,
}

// Clipboard tools' output may contain private data, including on failure.
async fn clipboard_output(args: &[&str]) -> Result<std::process::Output> {
    Ok(tokio::time::timeout(
        Duration::from_secs(10),
        Command::new("wl-paste")
            .args(args)
            .kill_on_drop(true)
            .output(),
    )
    .await??)
}

async fn wl_copy(args: &[&str], stdin: Stdio) -> Result<()> {
    let status = tokio::time::timeout(
        Duration::from_secs(10),
        Command::new("wl-copy")
            .args(args)
            .stdin(stdin)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .status(),
    )
    .await??;
    anyhow::ensure!(status.success(), "Could not copy to the host clipboard");
    Ok(())
}

async fn clipboard_read(mime: &str) -> Result<Vec<u8>> {
    let output = clipboard_output(&["--no-newline", "--type", mime]).await?;
    anyhow::ensure!(output.status.success(), "Could not read clipboard content");
    Ok(output.stdout)
}

impl SavedClipboard {
    async fn save() -> Result<Option<Self>> {
        let output = clipboard_output(&["--list-types"]).await?;
        if !output.status.success() {
            println!("Clipboard check not run: the host clipboard is empty or could not be read.");
            return Ok(None);
        }
        let types = String::from_utf8(output.stdout)
            .map_err(|_| anyhow::anyhow!("Clipboard format list is not UTF-8"))?;
        let types: Vec<_> = types.lines().collect();
        let restore = restorable(&types);
        if restore == Restore::Refuse {
            println!("Clipboard check not run: clipboard formats cannot be restored faithfully.");
            return Ok(None);
        }
        let bytes = match &restore {
            Restore::Clear => Vec::new(),
            Restore::Text(mime) | Restore::Single(mime) => clipboard_read(mime).await?,
            Restore::Refuse => bail!("Clipboard restoration was refused"),
        };
        let contents = bytes.clone();
        let directory = tokio::task::spawn_blocking(move || -> Result<TempDir> {
            use std::io::Write;
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            let directory = tempfile::Builder::new()
                .prefix("clipboard-")
                .tempdir_in(env!("CARGO_TARGET_TMPDIR"))?;
            std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))?;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(directory.path().join("saved"))?;
            file.write_all(&contents)?;
            Ok(directory)
        })
        .await??;
        Ok(Some(Self {
            directory,
            restore,
            sensitive: types.contains(&"x-kde-passwordManagerHint"),
            bytes,
        }))
    }

    async fn restore(&self) -> Result<()> {
        match &self.restore {
            Restore::Clear => {
                wl_copy(&restore_args(&self.restore, self.sensitive)?, Stdio::null()).await?;
            }
            Restore::Text(_) | Restore::Single(_) => {
                let args = restore_args(&self.restore, self.sensitive)?;
                let path = self.directory.path().join("saved");
                let input =
                    tokio::task::spawn_blocking(move || std::fs::File::open(path)).await??;
                wl_copy(&args, Stdio::from(input)).await?;
            }
            Restore::Refuse => bail!("Clipboard restoration was refused"),
        }
        match &self.restore {
            Restore::Clear => {
                let output = clipboard_output(&["--list-types"]).await?;
                anyhow::ensure!(output.stdout.is_empty(), "Restored clipboard is not empty");
            }
            Restore::Text(mime) | Restore::Single(mime) => {
                anyhow::ensure!(
                    clipboard_read(mime).await? == self.bytes,
                    "Restored clipboard differs from the saved content"
                );
            }
            Restore::Refuse => bail!("Clipboard restoration was refused"),
        }
        Ok(())
    }
}

struct ClipboardProbe {
    folder: PathBuf,
    sequence: u32,
}
impl ClipboardProbe {
    async fn execute(&mut self, script: &str) -> Result<Vec<u8>> {
        self.sequence += 1;
        let base = self.folder.join(self.sequence.to_string());
        clipboard_fs::write(base.with_extension("pending"), script).await?;
        clipboard_fs::rename(base.with_extension("pending"), base.with_extension("cmd")).await?;
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                match clipboard_fs::read_to_string(base.with_extension("rc")).await {
                    Ok(rc) => {
                        anyhow::ensure!(rc.trim() == "0", "Nested clipboard probe failed");
                        return Ok(clipboard_fs::read(base.with_extension("out")).await?);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                    Err(error) => return Err(error.into()),
                }
            }
        })
        .await?
    }
}

#[derive(PartialEq, Eq)]
enum ClipboardPhase {
    Takeover,
    HandBack,
}

async fn wait_clipboard_control(
    snapshots: &mut tokio::sync::watch::Receiver<keek::ipc::SessionSnapshot>,
    phase: ClipboardPhase,
) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let ready = match &snapshots.borrow_and_update().control {
                keek::ipc::ControlState::Takeover { .. } => phase == ClipboardPhase::Takeover,
                keek::ipc::ControlState::HandBack { .. } => phase == ClipboardPhase::HandBack,
                keek::ipc::ControlState::Verifier | keek::ipc::ControlState::HelpRequest { .. } => {
                    false
                }
            };
            if ready {
                return Ok::<_, anyhow::Error>(());
            }
            snapshots.changed().await?;
        }
    })
    .await?
}

// Tokio's fs feature is off, so filesystem work runs on blocking threads.
mod clipboard_fs {
    use std::path::PathBuf;
    pub async fn write(path: PathBuf, bytes: impl AsRef<[u8]>) -> std::io::Result<()> {
        let bytes = bytes.as_ref().to_vec();
        tokio::task::spawn_blocking(move || std::fs::write(path, bytes)).await?
    }
    pub async fn rename(from: PathBuf, to: PathBuf) -> std::io::Result<()> {
        tokio::task::spawn_blocking(move || std::fs::rename(from, to)).await?
    }
    pub async fn read(path: PathBuf) -> std::io::Result<Vec<u8>> {
        tokio::task::spawn_blocking(move || std::fs::read(path)).await?
    }
    pub async fn read_to_string(path: PathBuf) -> std::io::Result<String> {
        tokio::task::spawn_blocking(move || std::fs::read_to_string(path)).await?
    }
    pub async fn try_exists(path: PathBuf) -> std::io::Result<bool> {
        tokio::task::spawn_blocking(move || path.try_exists()).await?
    }
}

fn restore_args(restore: &Restore, sensitive: bool) -> Result<Vec<&str>> {
    match restore {
        Restore::Clear => Ok(vec!["--clear"]),
        Restore::Text(mime) | Restore::Single(mime) => {
            let mut args = vec!["--type", mime.as_str()];
            if sensitive {
                args.push("--sensitive");
            }
            Ok(args)
        }
        Restore::Refuse => bail!("Clipboard restoration was refused"),
    }
}

fn recovery_command(restore: &Restore, sensitive: bool, path: &Path) -> String {
    let Ok(args) = restore_args(restore, sensitive) else {
        return "Clipboard restoration was refused".into();
    };
    let quote = |value: &str| format!("'{}'", value.replace('\'', "'\"'\"'"));
    let mut command = String::from("wl-copy");
    for (index, arg) in args.into_iter().enumerate() {
        command.push(' ');
        if index != 1 {
            command.push_str(arg);
        } else {
            command.push_str(&quote(arg));
        }
    }
    match restore {
        Restore::Text(_) | Restore::Single(_) => {
            command.push_str(" < ");
            command.push_str(&quote(&path.join("saved").to_string_lossy()));
        }
        Restore::Clear | Restore::Refuse => {}
    }
    command
}
