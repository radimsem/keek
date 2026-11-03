async fn wait_for<T: Send + 'static>(
    what: &str,
    limit: Duration,
    probe: impl Fn() -> Option<T> + Clone + Send + 'static,
) -> Result<T> {
    let deadline = std::time::Instant::now() + limit;
    loop {
        let probe = probe.clone();
        if let Some(found) = tokio::task::spawn_blocking(probe).await? {
            return Ok(found);
        }
        anyhow::ensure!(
            std::time::Instant::now() < deadline,
            "timed out after {limit:?} waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

fn children(path: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(path)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .collect()
        })
        .unwrap_or_default()
}

/// Every filed file under `<videos>/keek/<project>/`, including the date folders' contents.
fn evidence_files(videos: &Path) -> Vec<PathBuf> {
    children(&videos.join("keek"))
        .iter()
        .flat_map(|project| children(project))
        .flat_map(|entry| {
            if entry.is_dir() {
                children(&entry)
            } else {
                vec![entry]
            }
        })
        .collect()
}

async fn incomplete_report(
    fixture: &Fixture,
    project: &str,
    limit: Duration,
) -> Result<(PathBuf, String)> {
    let videos = fixture.directory.path().join("videos");
    let prefix = format!("{project}_");
    wait_for(
        "the Incomplete Session's report",
        limit,
        move || {
            let report = evidence_files(&videos).into_iter().find(|path| {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                name.starts_with(&prefix) && name.ends_with("_incomplete_report.md")
            })?;
            // The report is created before it is written.
            let text = std::fs::read_to_string(&report).ok()?;
            text.contains("## Incomplete Session")
                .then_some((report, text))
        },
    )
    .await
}
