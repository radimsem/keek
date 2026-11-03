use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use rmcp::model::Tool;
use serde_json::Value;

pub const DRIVER_VERSION: &str = "0.38.0";
pub type DriverFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, DriverError>> + Send + 'a>>;

pub fn driver_path(home: &Path) -> PathBuf {
    home.join(format!(
        ".cua-driver/packages/releases/{DRIVER_VERSION}-x86_64-unknown-linux-gnu/cua-driver"
    ))
}

#[derive(Debug, thiserror::Error)]
pub enum DriverError {
    #[error(
        "Cua Driver could not complete its MCP handshake: {0}. Read session.err and hyprland.log in the Agent Session folder."
    )]
    Handshake(String),
    #[error(
        "Cua Driver could not list its tools: {0}. Check driver.log in the Agent Session folder."
    )]
    List(String),
    #[error(
        "Cua Driver could not call {tool}: {reason}. Check driver.log in the Agent Session folder."
    )]
    Call { tool: String, reason: String },
    #[error("Cua Driver arguments must be a JSON object. Pass the tool's named arguments.")]
    Arguments,
    #[error("Cua Driver returned invalid JSON: {0}. Check its pinned version.")]
    Json(#[from] serde_json::Error),
    #[error(
        "Cua Driver could not close its MCP connection: {0}. Check driver.log in the Agent Session folder."
    )]
    Close(String),
}

pub trait Driver: Send {
    fn end_cause(&self) -> Option<crate::checklist::IncompleteReason> {
        None
    }
    fn ended(&self) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(std::future::pending())
    }
    fn session_window(&self) -> Option<crate::compositor::WindowAddress> {
        None
    }
    fn list_tools(&mut self) -> DriverFuture<'_, Vec<Tool>>;
    fn call_tool<'a>(&'a mut self, name: &'a str, arguments: Value) -> DriverFuture<'a, Value>;
    fn close(&mut self) -> DriverFuture<'_, ()>;
}

pub struct CuaDriver {
    ended: tokio::sync::watch::Receiver<DriverState>,
    service: rmcp::service::RunningService<rmcp::service::RoleClient, ()>,
}
impl CuaDriver {
    pub async fn connect<R, W>(reader: R, writer: W) -> Result<Self, DriverError>
    where
        R: tokio::io::AsyncRead + Unpin + Send + 'static,
        W: tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        use rmcp::ServiceExt;
        let (ended_tx, ended) = tokio::sync::watch::channel(DriverState::Running);
        let reader = EndReader {
            reader,
            ended: ended_tx,
        };
        let service = ()
            .serve((reader, writer))
            .await
            .map_err(|error| DriverError::Handshake(error.to_string()))?;
        Ok(Self { service, ended })
    }
}
impl Driver for CuaDriver {
    fn ended(&self) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        let mut ended = self.ended.clone();
        Box::pin(async move {
            loop {
                if *ended.borrow_and_update() == DriverState::Ended {
                    return;
                }
                if ended.changed().await.is_err() {
                    return;
                }
            }
        })
    }
    fn list_tools(&mut self) -> DriverFuture<'_, Vec<Tool>> {
        Box::pin(async {
            self.service
                .list_all_tools()
                .await
                .map_err(|error| DriverError::List(error.to_string()))
        })
    }
    fn call_tool<'a>(&'a mut self, name: &'a str, arguments: Value) -> DriverFuture<'a, Value> {
        Box::pin(async move {
            let arguments = arguments.as_object().ok_or(DriverError::Arguments)?.clone();
            let result = self
                .service
                .call_tool(
                    rmcp::model::CallToolRequestParams::new(name.to_owned())
                        .with_arguments(arguments),
                )
                .await
                .map_err(|error| DriverError::Call {
                    tool: name.to_owned(),
                    reason: error.to_string(),
                })?;
            Ok(serde_json::to_value(result)?)
        })
    }
    fn close(&mut self) -> DriverFuture<'_, ()> {
        Box::pin(async {
            self.service
                .close()
                .await
                .map(|_| ())
                .map_err(|error| DriverError::Close(error.to_string()))
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DriverState {
    Running,
    Ended,
}
struct EndReader<R> {
    reader: R,
    ended: tokio::sync::watch::Sender<DriverState>,
}
impl<R> Drop for EndReader<R> {
    fn drop(&mut self) {
        self.ended.send_replace(DriverState::Ended);
    }
}
impl<R: tokio::io::AsyncRead + Unpin> tokio::io::AsyncRead for EndReader<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buffer: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let before = buffer.filled().len();
        let result = Pin::new(&mut self.reader).poll_read(cx, buffer);
        if matches!(&result, std::task::Poll::Ready(Err(_)))
            || matches!(&result, std::task::Poll::Ready(Ok(()))) && before == buffer.filled().len()
        {
            self.ended.send_replace(DriverState::Ended);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ListToolsResult,
    };
    use rmcp::service::{RequestContext, RoleServer};
    use rmcp::{ServerHandler, ServiceExt};
    use serde_json::json;

    struct Echo;
    impl ServerHandler for Echo {
        async fn list_tools(
            &self,
            _: Option<rmcp::model::PaginatedRequestParams>,
            _: RequestContext<RoleServer>,
        ) -> Result<ListToolsResult, rmcp::ErrorData> {
            Ok(ListToolsResult::with_all_items(vec![Tool::new(
                "echo",
                "echo fixture",
                serde_json::Map::new(),
            )]))
        }
        async fn call_tool(
            &self,
            params: CallToolRequestParams,
            _: RequestContext<RoleServer>,
        ) -> Result<CallToolResponse, rmcp::ErrorData> {
            Ok(CallToolResult::success(vec![ContentBlock::text(
                serde_json::to_string(&params.arguments).unwrap(),
            )])
            .into())
        }
    }

    #[tokio::test]
    async fn the_driver_lists_and_calls_tools_over_mcp() {
        let (near, far) = tokio::io::duplex(8192);
        let server =
            tokio::spawn(async move { Echo.serve(far).await.unwrap().waiting().await.unwrap() });
        let (read, write) = tokio::io::split(near);
        let mut driver = CuaDriver::connect(read, write).await.unwrap();
        assert_eq!(
            driver.list_tools().await.unwrap().first().unwrap().name,
            "echo"
        );
        let result = driver
            .call_tool("echo", json!({"hello":"world"}))
            .await
            .unwrap();
        assert!(result.to_string().contains("world"));
        driver.close().await.unwrap();
        server.await.unwrap();
    }
}
