use std::fs;
use std::io::{self, PipeWriter};
use std::os::fd::AsFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use wayland_client::globals::{
    BindError, GlobalError, GlobalList, GlobalListContents, registry_queue_init,
};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::{ConnectError, Connection, Dispatch, DispatchError, QueueHandle};
use wayland_protocols::wp::security_context::v1::client::{
    wp_security_context_manager_v1::WpSecurityContextManagerV1,
    wp_security_context_v1::WpSecurityContextV1,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "Can't reach the host compositor: {0}. Run this from a Wayland session with WAYLAND_DISPLAY set."
    )]
    Connect(#[from] ConnectError),
    #[error("Can't read the compositor's globals: {0}. Check that the compositor is responding.")]
    Globals(#[from] GlobalError),
    #[error(
        "The compositor refused wp_security_context_manager_v1: {0}. keek needs a compositor that offers it, such as Hyprland 0.56."
    )]
    Bind(#[from] BindError),
    #[error("The compositor closed the connection: {0}. Check its log for a protocol error.")]
    Dispatch(#[from] DispatchError),
    #[error("Can't set up the listener socket: {0}. Check that XDG_RUNTIME_DIR is writable.")]
    Io(#[from] io::Error),
}

struct State;

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WpSecurityContextManagerV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &WpSecurityContextManagerV1,
        _: <WpSecurityContextManagerV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WpSecurityContextV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &WpSecurityContextV1,
        _: <WpSecurityContextV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

/// One client connection the compositor treats as sandboxed.
///
/// The compositor keeps accepting on the listener until `listener_guard`
/// drops. Keep it until the client has spoken, because a connection still in
/// the backlog is lost when the listener goes away.
pub struct SecurityContextConnection {
    pub stream: UnixStream,
    pub listener_guard: PipeWriter,
}

pub fn open_security_context(
    runtime_dir: &Path,
    engine: &str,
    app_id: &str,
) -> Result<SecurityContextConnection, Error> {
    let conn = Connection::connect_to_env()?;
    let (globals, mut queue) = registry_queue_init::<State>(&conn)?;
    let qh = queue.handle();
    let manager: WpSecurityContextManagerV1 = globals.bind(&qh, 1..=1, ())?;

    let dir = private_dir(runtime_dir)?;
    let socket = dir.join("wayland");
    let listener = UnixListener::bind(&socket)?;
    let (close_reader, close_writer) = io::pipe()?;

    let context = manager.create_listener(listener.as_fd(), close_reader.as_fd(), &qh, ());
    context.set_sandbox_engine(engine.to_owned());
    context.set_app_id(app_id.to_owned());
    context.set_instance_id(std::process::id().to_string());
    context.commit();
    context.destroy();
    manager.destroy();
    queue.roundtrip(&mut State)?;

    let stream = UnixStream::connect(&socket)?;
    fs::remove_dir_all(&dir)?;

    Ok(SecurityContextConnection {
        stream,
        listener_guard: close_writer,
    })
}

/// Returns `interface version` lines, sorted, for the globals on a connection.
pub fn list_globals(conn: &Connection) -> Result<Vec<String>, Error> {
    let (globals, _queue): (GlobalList, _) = registry_queue_init::<State>(conn)?;
    let mut lines: Vec<String> = globals
        .contents()
        .clone_list()
        .into_iter()
        .map(|global| format!("{} {}", global.interface, global.version))
        .collect();
    lines.sort();
    Ok(lines)
}

fn private_dir(runtime_dir: &Path) -> io::Result<PathBuf> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    let dir = runtime_dir.join(format!("keek-sc-{}-{nanos}", std::process::id()));
    fs::create_dir(&dir)?;
    Ok(dir)
}
