#![allow(
    dead_code,
    reason = "The shared desktop fixture supports other live tests."
)]
mod common;
include!("common/live.rs");
include!("common/live_clipboard.rs");

#[test]
fn an_empty_clipboard_can_be_cleared_on_restore() {
    assert_eq!(restorable(&[]), Restore::Clear);
}

#[test]
fn text_aliases_can_be_restored_with_the_sensitive_hint() {
    for hint in [false, true] {
        let mut types = vec![
            "text/plain",
            "text/plain;charset=utf-8",
            "UTF8_STRING",
            "TEXT",
            "STRING",
        ];
        if hint {
            types.push("x-kde-passwordManagerHint");
        }
        assert_eq!(restorable(&types), Restore::Text("text/plain".into()));
    }
}

#[test]
fn one_image_type_can_be_restored() {
    assert_eq!(
        restorable(&["image/png"]),
        Restore::Single("image/png".into())
    );
}

#[test]
fn multiple_payload_formats_are_refused() {
    assert_eq!(restorable(&["image/png", "image/jpeg"]), Restore::Refuse);
    assert_eq!(restorable(&["text/plain", "text/html"]), Restore::Refuse);
}

#[test]
fn the_password_manager_hint_alone_is_refused() {
    assert_eq!(restorable(&["x-kde-passwordManagerHint"]), Restore::Refuse);
}

#[test]
fn the_utf8_string_alias_alone_can_be_restored() {
    assert_eq!(
        restorable(&["UTF8_STRING"]),
        Restore::Text("UTF8_STRING".into())
    );
}

#[test]
fn manual_recovery_preserves_the_mime_and_sensitive_hint() {
    for restore in [
        Restore::Text("text/plain;charset=utf-8".into()),
        Restore::Single("image/png".into()),
    ] {
        let mime = match &restore {
            Restore::Text(mime) | Restore::Single(mime) => mime,
            Restore::Clear | Restore::Refuse => unreachable!(),
        };
        for sensitive in [false, true] {
            let hint = if sensitive { " --sensitive" } else { "" };
            assert_eq!(
                recovery_command(&restore, sensitive, Path::new("/saved folder")),
                format!("wl-copy --type '{mime}'{hint} < '/saved folder/saved'")
            );
        }
    }
}

#[test]
fn manual_recovery_can_clear_an_empty_clipboard() {
    assert_eq!(
        recovery_command(&Restore::Clear, false, Path::new("/saved")),
        "wl-copy --clear"
    );
}

#[test]
fn manual_recovery_quotes_apostrophes_in_paths_and_types() {
    assert_eq!(
        recovery_command(
            &Restore::Single("application/x-'test".into()),
            false,
            Path::new("/a'b")
        ),
        "wl-copy --type 'application/x-'\"'\"'test' < '/a'\"'\"'b/saved'"
    );
}

#[tokio::test]
#[ignore = "requires the real Hyprland desktop and pinned Cua Driver"]
async fn live_a_takeover_keeps_the_host_clipboard_apart_from_the_nested_one() -> Result<()> {
    use keek::session::control::{Command as ControlCommand, Control, Controller};
    use std::sync::Arc;
    let _desktop = LIVE_DESKTOP.lock().await;
    let Some(saved) = SavedClipboard::save().await? else {
        return Ok(());
    };
    println!(
        "Saved the host clipboard to {} for the restore.",
        saved.directory.path().display()
    );
    let before = host_state().await?;
    let mut fixture = Fixture::start().await?;
    let mut controller_task = None;
    let mut control_handle = None;
    let mut session_folder = None;
    let result = async {
        let mut windows = keek::hypr::subscribe().await?;
        let worktree = fixture.worktree_with_start("clip", "bash probe.sh").await?;
        clipboard_fs::write(
            worktree.join("probe.sh"),
            include_str!("fixtures/clipboard_probe.sh"),
        )
        .await?;
        let session = fixture.open("clip", &worktree).await?;
        let folder = fixture
            .directory
            .path()
            .join("cache/keek/sessions")
            .join(&session);
        session_folder = Some(folder.clone());
        let address = tokio::time::timeout(Duration::from_secs(30), async {
            while let Some(line) = windows.next_line().await? {
                if let Some(keek::hypr::HyprEvent::OpenWindow { address, class, .. }) =
                    keek::hypr::parse(&line)
                    && class == "aquamarine"
                {
                    return Ok::<_, anyhow::Error>(format!("0x{address}"));
                }
            }
            bail!("No Session Window opened")
        })
        .await??;
        let probe_folder = folder.join("probe");
        tokio::time::timeout(Duration::from_secs(30), async {
            while !clipboard_fs::try_exists(probe_folder.join("ready")).await? {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Ok::<_, anyhow::Error>(())
        })
        .await??;
        let mut probe = ClipboardProbe {
            folder: probe_folder,
            sequence: 0,
        };
        let suffix = std::process::id();
        let n = format!("keek-made-up-N-{suffix}");
        let a = format!("keek-made-up-A-{suffix}");
        let b = format!("keek-made-up-B-{suffix}");
        probe
            .execute(&format!("wl-copy --sensitive --type text/plain -- '{n}'"))
            .await?;
        anyhow::ensure!(
            probe
                .execute("wl-paste --no-newline --type text/plain")
                .await?
                == n.as_bytes(),
            "Nested positive control N failed"
        );
        wl_copy(
            &["--sensitive", "--type", "text/plain", "--", &a],
            Stdio::null(),
        )
        .await?;
        anyhow::ensure!(
            clipboard_read("text/plain").await? == a.as_bytes(),
            "Host positive control A failed"
        );
        let nested = probe
            .execute("wl-paste --no-newline --type text/plain")
            .await?;
        anyhow::ensure!(
            !nested.windows(a.len()).any(|v| v == a.as_bytes()),
            "Host A reached nested clipboard before Takeover"
        );
        let (control, receiver) = Control::new();
        let (events, mut snapshots) = tokio::sync::watch::channel(keek::ipc::SessionSnapshot {
            control: keek::ipc::ControlState::Verifier,
            state: keek::ipc::SessionState::Open,
            checks: vec![],
            last_action: None,
        });
        let runner = Arc::new(keek::process::HostRunner);
        let controller = Controller {
            control: control.clone(),
            runner: runner.clone(),
            compositor: Arc::new(keek::compositor::HostCompositor::new(runner)),
            clock: Arc::new(keek::clock::MonotonicClock),
            events,
            opened: std::time::Instant::now(),
            window: Some(keek::compositor::WindowAddress(address.clone())),
            project: "clip".into(),
        };
        controller_task = Some(tokio::spawn(
            controller.run(receiver, Box::pin(std::future::pending())),
        ));
        control_handle = Some(control.clone());
        control.commands.send(ControlCommand::TakeControl).await?;
        wait_clipboard_control(&mut snapshots, ClipboardPhase::Takeover).await?;
        println!("Takeover state: {:?}", snapshots.borrow().control);
        anyhow::ensure!(
            host_state().await?.get("address").and_then(Value::as_str) == Some(address.as_str()),
            "Session Window is not active during Takeover"
        );
        let nested = probe
            .execute("wl-paste --no-newline --type text/plain")
            .await?;
        anyhow::ensure!(
            !nested.windows(a.len()).any(|v| v == a.as_bytes()),
            "Host A reached nested clipboard during Takeover"
        );
        probe
            .execute(&format!("wl-copy --sensitive --type text/plain -- '{b}'"))
            .await?;
        anyhow::ensure!(
            probe
                .execute("wl-paste --no-newline --type text/plain")
                .await?
                == b.as_bytes(),
            "Nested positive control B failed"
        );
        let host = clipboard_read("text/plain").await?;
        anyhow::ensure!(
            host == a.as_bytes() && !host.windows(b.len()).any(|v| v == b.as_bytes()),
            "Nested B changed host clipboard during Takeover"
        );
        control.commands.send(ControlCommand::Done).await?;
        wait_clipboard_control(&mut snapshots, ClipboardPhase::HandBack).await?;
        let host = clipboard_read("text/plain").await?;
        anyhow::ensure!(
            host == a.as_bytes() && !host.windows(b.len()).any(|v| v == b.as_bytes()),
            "Host clipboard changed after Done"
        );
        anyhow::ensure!(
            before == host_state().await?,
            "Host focus or workspace changed after Done"
        );
        println!("Clipboard separation checks passed before, during and after Takeover");
        Ok::<_, anyhow::Error>(())
    }
    .await;
    if let Some(control) = control_handle {
        control.stop().await;
    }
    let stopped = async {
        if let Some(task) = controller_task {
            task.await?;
        }
        Ok::<_, anyhow::Error>(())
    }
    .await;
    let restored = saved.restore().await;
    let cleanup = fixture.finish().await;
    let gone = async {
        if let Some(folder) = session_folder {
            assert_processes_gone(&folder).await?;
        }
        anyhow::ensure!(
            before == host_state().await?,
            "Host focus or workspace changed after cleanup"
        );
        Ok::<_, anyhow::Error>(())
    }
    .await;
    let removed = if restored.is_err() {
        let recovery = recovery_command(&saved.restore, saved.sensitive, saved.directory.path());
        let kept = saved.directory.keep();
        Err(anyhow::anyhow!(
            "Kept the saved host clipboard in {}. Restore it with `{recovery}`, then delete the folder.",
            kept.display()
        ))
    } else {
        tokio::task::spawn_blocking(move || saved.directory.close())
            .await
            .map_err(anyhow::Error::from)
            .and_then(|r| r.map_err(Into::into))
    };
    let mut errors = Vec::new();
    for outcome in [result, stopped, restored, cleanup, gone, removed] {
        if let Err(error) = outcome {
            errors.push(format!("{error:#}"));
        }
    }
    match run("hyprctl", &["version", "-j"], None).await {
        Ok(version) => println!("Hyprland version: {version}"),
        Err(error) => errors.push(format!("Could not read Hyprland version: {error:#}")),
    }
    anyhow::ensure!(
        errors.is_empty(),
        "Clipboard check failed: {}",
        errors.join("; ")
    );
    Ok(())
}
