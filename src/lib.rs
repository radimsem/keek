pub mod build_id;
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
pub mod lan_block;
pub mod mcp;
pub mod omarchy_toggles;
pub mod paths;
pub mod process;
pub mod project;
pub mod project_prepare;
pub mod registry;
mod reminder;
pub mod report;
pub mod sandbox;
pub mod security_context;
pub mod session;
pub mod session_factory;
pub mod session_folder;
pub mod takeover_clipboard;
pub mod timeline;
pub mod title_card;
pub mod toolchain;
pub mod verifier_protocol;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

mod session_evidence;

pub mod open_progress;

mod session_project;

pub mod credential_scrub;
pub mod credentials;
pub mod hypr;
pub mod revision;
mod session_messages;
pub mod slots;
pub mod viewer;
mod window_placement;

mod session_journal;

pub mod attempts;
pub mod session_frame;

mod window_guard;

pub mod test_files;
