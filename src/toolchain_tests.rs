use super::*;
use std::sync::Mutex;

type Script = Box<dyn Fn(&[String]) -> MiseOutput + Send + Sync>;
struct FakeMise {
    calls: Mutex<Vec<String>>,
    script: Script,
}
impl FakeMise {
    fn new(script: impl Fn(&[String]) -> MiseOutput + Send + Sync + 'static) -> Self {
        Self {
            calls: Mutex::new(vec![]),
            script: Box::new(script),
        }
    }
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}
impl Mise for FakeMise {
    fn run<'a>(&'a self, _: &'a Path, args: &'a [String], _: Option<&'a Path>) -> MiseFuture<'a> {
        self.calls.lock().unwrap().push(args.join(" "));
        let output = (self.script)(args);
        Box::pin(async move { Ok(output) })
    }
}
fn ok(stdout: &str) -> MiseOutput {
    MiseOutput {
        success: true,
        stdout: stdout.into(),
    }
}
fn failed() -> MiseOutput {
    MiseOutput {
        success: false,
        stdout: String::new(),
    }
}
fn pins(node: &str) -> Toolchain {
    Toolchain {
        node: Some(node.into()),
        ..Toolchain::default()
    }
}

async fn resolved(mise: &FakeMise, store: &Path, toolchain: &Toolchain) -> Toolchains {
    let log = store.join("toolchain.log");
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        resolve(mise, store, toolchain, &log),
    )
    .await
    .unwrap()
    .unwrap()
}

#[tokio::test]
async fn a_version_already_in_the_store_is_not_installed_again() {
    let store = tempfile::tempdir().unwrap();
    let installs = store.path().join("data/installs/node/24.11.1");
    let bin = installs.join("bin");
    let mise = FakeMise::new(move |args| match args[0].as_str() {
        "where" => ok(&format!("{}\n", installs.display())),
        "bin-paths" => ok(&format!("{}\n", bin.display())),
        other => panic!("unexpected mise {other}"),
    });
    let toolchains = resolved(&mise, store.path(), &pins("24")).await;
    assert_eq!(mise.calls(), ["where node@24", "bin-paths node@24.11.1"]);
    let [node] = toolchains.tools.as_slice() else {
        panic!("{toolchains:?}");
    };
    assert_eq!((node.tool, node.version.as_str()), (Tool::Node, "24.11.1"));
    assert_eq!(node.fetch, Fetch::Present);
}

#[tokio::test]
async fn a_missing_version_is_installed_into_the_store_once() {
    let store = tempfile::tempdir().unwrap();
    let installs = store.path().join("data/installs/node/24.11.1");
    let bin = installs.join("bin");
    let installed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mise = FakeMise::new(move |args| match args[0].as_str() {
        "where" if installed.load(std::sync::atomic::Ordering::SeqCst) => {
            ok(&format!("{}\n", installs.display()))
        }
        "where" => failed(),
        "install" => {
            installed.store(true, std::sync::atomic::Ordering::SeqCst);
            ok("")
        }
        "bin-paths" => ok(&format!("{}\n", bin.display())),
        other => panic!("unexpected mise {other}"),
    });
    let first = resolved(&mise, store.path(), &pins("24")).await;
    let second = resolved(&mise, store.path(), &pins("24")).await;
    assert_eq!(first.tools[0].fetch, Fetch::Installed);
    assert_eq!(second.tools[0].fetch, Fetch::Present);
    let installs: Vec<String> = mise
        .calls()
        .into_iter()
        .filter(|call| call.starts_with("install"))
        .collect();
    assert_eq!(installs, ["install node@24"]);
}

#[tokio::test]
async fn a_failed_install_names_the_tool_pin_and_log() {
    let store = tempfile::tempdir().unwrap();
    let mise = FakeMise::new(|_| failed());
    let log = store.path().join("toolchain.log");
    let error = resolve(&mise, store.path(), &pins("99"), &log)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("node 99") && error.contains("toolchain.log"),
        "{error}"
    );
}

#[tokio::test]
async fn bin_dirs_outside_the_store_are_left_off_path() {
    let store = tempfile::tempdir().unwrap();
    let installs = store.path().join("data/installs/node/24.11.1");
    let inside = installs.join("bin");
    let mise = FakeMise::new(move |args| match args[0].as_str() {
        "where" => ok(&format!("{}\n", installs.display())),
        _ => ok(&format!("/usr/local/bin\n{}\n", inside.display())),
    });
    let toolchains = resolved(&mise, store.path(), &pins("24")).await;
    assert_eq!(
        toolchains.path_dirs(),
        [store.path().join("data/installs/node/24.11.1/bin")]
    );
}

#[tokio::test]
async fn no_pins_run_no_mise() {
    let store = tempfile::tempdir().unwrap();
    let mise = FakeMise::new(|_| panic!("mise ran"));
    let toolchains = resolved(&mise, store.path(), &Toolchain::default()).await;
    assert!(toolchains.tools.is_empty());
}

#[test]
fn host_mise_keeps_every_mise_and_rust_home_inside_the_store() {
    let store = Path::new("/cache/keek/toolchains");
    let command = HostMise::command(store, &["install".to_owned(), "node@24".to_owned()]);
    let command = command.as_std();
    assert_eq!(command.get_program(), "/usr/bin/mise");
    assert_eq!(
        command.get_current_dir(),
        Some(store.join("work").as_path())
    );
    let envs: Vec<(String, String)> = command
        .get_envs()
        .map(|(name, value)| {
            (
                name.to_string_lossy().into_owned(),
                value.unwrap().to_string_lossy().into_owned(),
            )
        })
        .collect();
    for name in [
        "HOME",
        "MISE_DATA_DIR",
        "MISE_CONFIG_DIR",
        "MISE_CACHE_DIR",
        "MISE_STATE_DIR",
        "MISE_SYSTEM_CONFIG_DIR",
        "MISE_GLOBAL_CONFIG_FILE",
        "MISE_RUSTUP_HOME",
        "MISE_CARGO_HOME",
    ] {
        let (_, value) = envs
            .iter()
            .find(|(key, _)| key == name)
            .unwrap_or_else(|| panic!("{name} is not set"));
        assert!(
            value.starts_with("/cache/keek/toolchains/"),
            "{name}={value}"
        );
    }
    assert!(
        envs.iter()
            .all(|(_, value)| !value.contains("/.config/mise") && !value.contains(".rustup"))
    );
}

#[test]
fn toolchain_pins_refuse_unknown_tools_and_odd_versions() {
    let parse = |text: &str| crate::project::Profile::parse(Path::new("demo.toml"), text);
    let profile = parse("start = \"x\"\n[toolchain]\nnode = \"24\"\npnpm = \"9.12.0\"").unwrap();
    assert_eq!(profile.toolchain.node.as_deref(), Some("24"));
    assert!(parse("start = \"x\"\n[toolchain]\nruby = \"3\"").is_err());
    for pin in ["", "24 && rm", "../24", "latest@x"] {
        let error = parse(&format!("start = \"x\"\n[toolchain]\nnode = {pin:?}"))
            .unwrap_err()
            .to_string();
        assert!(error.contains("toolchain node"), "{pin}: {error}");
    }
}
