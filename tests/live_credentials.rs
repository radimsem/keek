#![allow(
    dead_code,
    reason = "The shared desktop fixture supports other live tests."
)]
mod common;
include!("common/live.rs");

#[tokio::test]
#[ignore = "requires real Hyprland, bwrap, pasta and pinned Cua Driver"]
async fn live_credentials_reach_the_app_and_leave_no_text_evidence() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let _desktop = LIVE_DESKTOP.lock().await;
    let mut fixture = Fixture::start().await?;
    let result = async {
        let value = "pa\"ss\\w&rd 1";
        let worktree = fixture.worktree("credentials").await?;
        let config = fixture.directory.path().join("config/keek/projects");
        std::fs::create_dir_all(&config)?;
        let path = config.join("credentials.env");
        std::fs::write(&path, format!("KEEK_TEST_PASSWORD={value}\n"))?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))?;
        std::fs::write(config.join("credentials.toml"), "[window]\nwidth=390\nheight=844\n")?;
        std::fs::write(worktree.join("index.html"), "<title>keek fixture</title><input style='position:absolute;inset:0;width:100%;height:100%' oninput='console.log(this.value)'>")?;
        std::fs::write(worktree.join(".keek/setup.sh"), r#"#!/bin/bash
case "$1" in
install) exit 0 ;;
start)
tr '\0' ' ' < /proc/1/cmdline > "$KEEK_SESSION/init-cmdline"
printf '%s' "$KEEK_TEST_PASSWORD" > "$KEEK_SESSION/app-env"
exec chromium --enable-logging=stderr --ozone-platform=wayland --kiosk --no-first-run --no-default-browser-check --password-store=basic --user-data-dir="$HOME/chromium" file:///keek/worktree/index.html
;;
esac
"#)?;
        run("git", &["add", "index.html", ".keek/setup.sh"], Some(&worktree)).await?;
        let opened=call(&fixture.client,"open_session",json!({"project":"credentials","worktree":worktree,"goal":"verify Test Credentials","checks":["input receives Test Credentials"]})).await?;
        anyhow::ensure!(opened["credentials"]==json!(["KEEK_TEST_PASSWORD"]),"credential names differ");
        anyhow::ensure!(!opened.to_string().contains(value),"open result contains a value");
        anyhow::ensure!(opened["warnings"].as_array().is_some_and(|items|items.iter().any(|w|w.as_str().is_some_and(|s|s.contains("644")&&s.contains("chmod 600")))),"mode warning missing");
        let session=opened["session"].as_str().context("session missing")?.to_owned();fixture.sessions.push(session.clone());
        println!("open_session: credential name only and mode warning verified");
        let cache=fixture.directory.path().join("cache/keek/sessions").join(&session);
        anyhow::ensure!(std::fs::read_to_string(cache.join("app-env"))?==value,"App Under Test environment differs");
        println!("App Under Test: injected environment verified");
        let pid=fixture.click(&session,1).await?;
        let typed=call(&fixture.client,"type_text",json!({"session":session,"pid":pid,"text":"${KEEK_TEST_PASSWORD}"})).await?;
        anyhow::ensure!(!typed.to_string().contains(value),"type_text result contains a value");
        let mut arrived=false;
        for _ in 0..100 {
            if std::fs::read_to_string(cache.join("start.log")).unwrap_or_default().contains(value) {arrived=true;break;}
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        anyhow::ensure!(arrived,"page did not log the substituted input");
        println!("type_text: page received the value and tool result is masked");
        let readable=command_lines(&cache.to_string_lossy());
        anyhow::ensure!(!readable.is_empty(),"no readable host cmdline names the session folder, so the scan proves nothing");
        anyhow::ensure!(command_lines(value).is_empty(),"a Test Credential value sits on a host command line");
        println!("host cmdlines: every readable one checked, {} name the session folder, none carries the value",readable.len());
        let init=std::fs::read_to_string(cache.join("init-cmdline"))?;
        anyhow::ensure!(init.contains("--args")&&!init.contains(value),"sandbox pid 1 lacks the arguments file or carries the value: {init}");
        println!("sandbox /proc/1/cmdline: carries --args, not the value");
        let records=fixture.close(&session).await?;
        let evidence=records.report.parent().and_then(Path::parent).context("Project Evidence folder missing")?;
        let status=Command::new("rg").args(["--no-ignore","--hidden","-F","--"]).arg(value).arg(evidence).stdout(Stdio::null()).stderr(Stdio::null()).status().await?;
        anyhow::ensure!(status.code()==Some(1),"credential remains in filed Evidence or rg failed");
        let actions=std::fs::read_dir(evidence.join(&*records.report.parent().and_then(Path::file_name).context("date folder missing")?.to_string_lossy()))?.filter_map(Result::ok).map(|entry|entry.path()).find(|path|path.to_string_lossy().ends_with("_actions")).context("filed Action Log missing")?;
        let masked=Command::new("rg").args(["-l","-F","--","[masked]"]).arg(&actions).output().await?;
        anyhow::ensure!(masked.status.code()==Some(0),"the filed Action Log recorded no masked Action");
        println!("filed Evidence: rg returned 1; [masked] in {}",String::from_utf8_lossy(&masked.stdout).trim());
        anyhow::ensure!(!cache.exists(),"session cache remains");
        println!("session cache: removed after filing");
        Ok::<(),anyhow::Error>(())
    }.await;
    let cleanup = fixture.finish().await;
    result.and(cleanup)
}

fn command_lines(needle: &str) -> Vec<String> {
    std::fs::read_dir("/proc")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.is_empty() || !name.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            let bytes = std::fs::read(format!("/proc/{name}/cmdline")).ok()?;
            bytes
                .windows(needle.len())
                .any(|window| window == needle.as_bytes())
                .then_some(name)
        })
        .collect()
}
