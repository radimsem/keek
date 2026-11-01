use std::path::Path;
#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error("{program} timed out after 30 s. Check the command and try again.")]
    Timeout { program: String },
    #[error("could not run {program}: {source}. Check that it is installed.")]
    Io {
        program: String,
        source: std::io::Error,
    },
    #[error("{program} failed: {message}. Check the command and try again.")]
    Failed { program: String, message: String },
}
pub type ProcessFuture<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, ProcessError>> + Send + 'a>>;
pub trait ProcessRunner: Send + Sync {
    fn output<'a>(
        &'a self,
        program: &'a str,
        args: &'a [&'a str],
        cwd: Option<&'a Path>,
    ) -> ProcessFuture<'a>;
}
pub struct HostRunner;
impl ProcessRunner for HostRunner {
    fn output<'a>(
        &'a self,
        program: &'a str,
        args: &'a [&'a str],
        cwd: Option<&'a Path>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<String, ProcessError>> + Send + 'a>,
    > {
        Box::pin(async move {
            let mut command = tokio::process::Command::new(program);
            command.args(args).kill_on_drop(true);
            if let Some(cwd) = cwd {
                command.current_dir(cwd);
            }
            let out = tokio::time::timeout(std::time::Duration::from_secs(30), command.output())
                .await
                .map_err(|_| ProcessError::Timeout {
                    program: program.into(),
                })?
                .map_err(|source| ProcessError::Io {
                    program: program.into(),
                    source,
                })?;
            if !out.status.success() {
                return Err(ProcessError::Failed {
                    program: program.into(),
                    message: format!(
                        "{}: {}",
                        out.status,
                        String::from_utf8_lossy(if out.stderr.is_empty() {
                            &out.stdout
                        } else {
                            &out.stderr
                        })
                        .trim()
                    ),
                });
            }
            Ok(String::from_utf8_lossy(&out.stdout).trim().into())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test(start_paused = true)]
    async fn host_runner_times_out_a_stuck_process() {
        let task = tokio::spawn(async { HostRunner.output("sh", &["-c", "sleep 60"], None).await });
        tokio::task::yield_now().await;
        tokio::time::advance(std::time::Duration::from_secs(31)).await;
        let result = tokio::time::timeout(std::time::Duration::from_secs(1), task).await;
        assert!(matches!(result, Ok(Ok(Err(_)))), "{result:?}");
    }
    #[tokio::test]
    async fn failed_process_reports_output_and_exit_status() {
        for (script, expected) in [
            ("printf stdout-error; exit 7", "stdout-error"),
            ("printf stderr-error >&2; exit 9", "stderr-error"),
        ] {
            let error = HostRunner
                .output("sh", &["-c", script], None)
                .await
                .unwrap_err()
                .to_string();
            assert!(
                error.contains(expected) && error.contains("exit status"),
                "{error}"
            );
        }
    }
}
