pub mod clock;
pub mod daemon;
pub mod ipc;
pub mod paths;
pub mod session_folder;
pub mod timeline;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
