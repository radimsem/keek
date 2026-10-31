pub mod clock;
pub mod daemon;
pub mod daemon_client;
pub mod ipc;
pub mod mcp;
pub mod paths;
pub mod session_folder;
pub mod timeline;
pub mod verifier_protocol;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
