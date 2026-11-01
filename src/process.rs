use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};

const STALL_LIMIT: Duration = Duration::from_secs(60);
#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error("{program} timed out after 30 s. Check the command and try again.")]
    Timeout { program: String },
    #[error("{program} produced no stdout for 60 s. Check the command and try again.")]
    Stalled { program: String },
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
pub type BytesFuture<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<u8>, ProcessError>> + Send + 'a>>;
pub trait ProcessRunner: Send + Sync {
    fn output_bytes<'a>(
        &'a self,
        program: &'a str,
        args: &'a [&'a str],
        cwd: Option<&'a Path>,
    ) -> BytesFuture<'a> {
        Box::pin(async move {
            self.output(program, args, cwd)
                .await
                .map(String::into_bytes)
        })
    }
    fn output<'a>(
        &'a self,
        program: &'a str,
        args: &'a [&'a str],
        cwd: Option<&'a Path>,
    ) -> ProcessFuture<'a>;
    /// Runs a long job, calling `on_stdout` per stdout line, and returns stderr. `HostRunner` sets
    /// no overall time limit and fails if stdout stays silent for `STALL_LIMIT`. The default body
    /// is for fakes and delegates to `output`.
    fn stream<'a>(
        &'a self,
        program: &'a str,
        args: &'a [&'a str],
        _on_stdout: &'a mut (dyn FnMut(&str) + Send),
    ) -> ProcessFuture<'a> {
        self.output(program, args, None)
    }
}
pub struct HostRunner;
impl ProcessRunner for HostRunner {
    fn stream<'a>(
        &'a self,
        program: &'a str,
        args: &'a [&'a str],
        on_stdout: &'a mut (dyn FnMut(&str) + Send),
    ) -> ProcessFuture<'a> {
        Box::pin(async move {
            let io_error = |source| ProcessError::Io {
                program: program.into(),
                source,
            };
            let mut child = tokio::process::Command::new(program)
                .args(args)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .map_err(io_error)?;
            let stdout = child
                .stdout
                .take()
                .ok_or_else(|| io_error(std::io::Error::other("command stdout was not piped")))?;
            let read_stdout = async {
                let mut lines = BufReader::new(stdout).lines();
                let mut deadline = tokio::time::Instant::now() + STALL_LIMIT;
                while let Some(line) = tokio::time::timeout_at(deadline, lines.next_line())
                    .await
                    .map_err(|_| ProcessError::Stalled {
                        program: program.into(),
                    })?
                    .map_err(io_error)?
                {
                    on_stdout(&line);
                    deadline = tokio::time::Instant::now() + STALL_LIMIT;
                }
                Ok::<_, ProcessError>(deadline)
            };
            let collect_stderr = async { child.wait_with_output().await.map_err(io_error) };
            tokio::pin!(read_stdout, collect_stderr);
            let output = tokio::select! {
                output = &mut collect_stderr => {
                    read_stdout.await?;
                    output?
                }
                deadline = &mut read_stdout => {
                    tokio::time::timeout_at(deadline?, collect_stderr)
                        .await
                        .map_err(|_| ProcessError::Stalled {
                            program: program.into(),
                        })??
                }
            };
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            if !output.status.success() {
                return Err(ProcessError::Failed {
                    program: program.into(),
                    message: format!("{}: {stderr}", output.status),
                });
            }
            Ok(stderr)
        })
    }
    fn output<'a>(
        &'a self,
        program: &'a str,
        args: &'a [&'a str],
        cwd: Option<&'a Path>,
    ) -> ProcessFuture<'a> {
        Box::pin(async move {
            self.output_bytes(program, args, cwd)
                .await
                .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_owned())
        })
    }
    fn output_bytes<'a>(
        &'a self,
        program: &'a str,
        args: &'a [&'a str],
        cwd: Option<&'a Path>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Vec<u8>, ProcessError>> + Send + 'a>,
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
            Ok(out.stdout)
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
    #[tokio::test(start_paused = true)]
    async fn host_runner_stalls_when_stdout_stops_progressing() {
        let task = tokio::spawn(async {
            HostRunner
                .stream("sh", &["-c", "exec sleep 120"], &mut |_| {})
                .await
        });
        tokio::task::yield_now().await;
        tokio::time::advance(std::time::Duration::from_secs(61)).await;
        let result = tokio::time::timeout(std::time::Duration::from_secs(1), task).await;
        assert!(
            matches!(result, Ok(Ok(Err(ProcessError::Stalled { .. })))),
            "{result:?}"
        );
    }
    #[tokio::test(start_paused = true)]
    async fn host_runner_stalls_when_stdout_closes_before_the_process_exits()
    -> Result<(), std::io::Error> {
        let directory = tempfile::tempdir()?;
        let marker = directory.path().join("stdout-closed");
        let marker_arg = marker.to_string_lossy().into_owned();
        let mut task = tokio::spawn(async move {
            HostRunner
                .stream(
                    "sh",
                    &[
                        "-c",
                        "exec 1>&-; printf ready > \"$1\"; exec sleep 120",
                        "sh",
                        &marker_arg,
                    ],
                    &mut |_| {},
                )
                .await
        });
        let started = std::time::Instant::now();
        while !marker.exists() && started.elapsed() < Duration::from_secs(5) {
            tokio::task::yield_now().await;
        }
        let stdout_closed = marker.exists();
        if !stdout_closed {
            task.abort();
            let _ = (&mut task).await;
        }
        assert!(stdout_closed, "child did not close stdout");
        tokio::time::advance(Duration::from_secs(61)).await;
        let result = tokio::time::timeout(Duration::from_secs(1), &mut task).await;
        if result.is_err() {
            task.abort();
            let _ = task.await;
        }
        assert!(
            matches!(result, Ok(Ok(Err(ProcessError::Stalled { .. })))),
            "{result:?}"
        );
        Ok(())
    }
    #[tokio::test]
    async fn host_runner_streams_stdout_lines_while_draining_stderr() -> Result<(), ProcessError> {
        let mut lines = Vec::new();
        let stderr = HostRunner
            .stream(
                "sh",
                &[
                    "-c",
                    "i=0; while [ \"$i\" -lt 10000 ]; do printf 'stderr-line\\n' >&2; i=$((i + 1)); done; printf 'first\\nsecond\\n'",
                ],
                &mut |line| lines.push(line.to_owned()),
            )
            .await?;
        assert_eq!(lines, ["first", "second"]);
        assert_eq!(stderr, "stderr-line\n".repeat(10000).trim_end());
        Ok(())
    }
    #[tokio::test]
    async fn failed_stream_reports_stderr_and_exit_status() {
        let error = HostRunner
            .stream(
                "sh",
                &[
                    "-c",
                    "printf 'progress\\n'; printf stderr-error >&2; exit 9",
                ],
                &mut |_| {},
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("stderr-error") && error.contains("exit status: 9"),
            "{error}"
        );
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
    #[tokio::test]
    async fn project_file_output_preserves_raw_bytes_and_trailing_newlines() {
        let bytes = HostRunner
            .output_bytes("sh", &["-c", "printf 'name\\n\\000'"], None)
            .await
            .unwrap();
        assert_eq!(bytes, b"name\n\0");
    }
}
