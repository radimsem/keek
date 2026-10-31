use std::time::Duration;

use anyhow::{Context, Result};
use rmcp::handler::server::wrapper::Json;
use rmcp::model::{Implementation, ServerCapabilities, ServerConfig};
use rmcp::{ErrorData, ServerHandler, ServiceExt, tool, tool_handler, tool_router};

use crate::VERSION;
use crate::daemon_client::{self, DaemonClient, ProcessStarter};
use crate::ipc::DaemonStatus;
use crate::paths::Paths;
use crate::verifier_protocol::VERIFIER_PROTOCOL;

const DAEMON_START_TIMEOUT: Duration = Duration::from_secs(2);

/// Reaches the daemon, then serves MCP over stdio until the coding agent leaves.
///
/// # Errors
///
/// Fails when the daemon can't be reached, or MCP can't be served on stdio.
pub async fn serve() -> Result<()> {
    let paths = Paths::from_env()?;
    let executable = std::env::current_exe().context("could not find the keek executable")?;
    let daemon = daemon_client::connect_or_start(
        &paths,
        &mut ProcessStarter::new(executable),
        DAEMON_START_TIMEOUT,
    )
    .await?;

    KeekServer { daemon }
        .serve(rmcp::transport::stdio())
        .await
        .context("could not start the MCP server")?
        .waiting()
        .await
        .context("the MCP server stopped unexpectedly")?;
    Ok(())
}

struct KeekServer {
    daemon: DaemonClient,
}

#[tool_router]
impl KeekServer {
    #[tool(description = "Report the keek daemon's version, pid and open Agent Session count.")]
    async fn keek_status(&self) -> Result<Json<DaemonStatus>, ErrorData> {
        self.daemon.status().await.map(Json).map_err(|error| {
            let error = anyhow::Error::from(error);
            ErrorData::internal_error(format!("keek_status failed: {error:#}"), None)
        })
    }
}

#[tool_handler]
impl ServerHandler for KeekServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("keek", VERSION))
            .with_instructions(VERIFIER_PROTOCOL)
    }
}
