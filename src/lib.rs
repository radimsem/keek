pub mod catalog;
pub mod checklist;
pub mod clock;
pub mod compositor;
pub mod config;
pub mod connect_exec;
pub mod daemon;
pub mod daemon_client;
pub mod driver;
pub mod editor;
pub mod evidence;
pub mod ipc;
pub mod mcp;
pub mod paths;
pub mod process;
pub mod project;
pub mod registry;
pub mod report;
pub mod sandbox;
pub mod security_context;
pub mod session;
pub mod session_factory;
pub mod session_folder;
pub mod timeline;
pub mod verifier_protocol;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

mod session_evidence;
