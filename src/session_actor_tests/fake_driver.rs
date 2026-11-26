use super::*;
/// Cua refuses any call with this element token.
pub(super) const REFUSED_TOKEN: &str = "refused-token";
#[derive(Clone, Copy)]
pub(super) enum Screenshot {
    Ready,
    Image(&'static str, &'static str),
    NoImage,
    Held,
    Failed,
    ReportedFailure,
}
#[derive(Clone, Copy)]
pub(super) enum WindowsReply {
    Listed,
    Failed,
    ReportedFailure,
}
pub(super) struct FakeDriver {
    pub(super) window_state: Arc<Mutex<Value>>,
    pub(super) windows: Arc<Mutex<WindowsReply>>,
    pub(super) cause: Arc<Mutex<Option<IncompleteReason>>>,
    pub(super) screenshot: Arc<Mutex<Screenshot>>,
    pub(super) ended: watch::Receiver<()>,
    pub(super) id: String,
    pub(super) clock: Arc<FakeClock>,
    pub(super) calls: Arc<Mutex<Vec<(String, Value)>>>,
    pub(super) closed: Arc<AtomicBool>,
    pub(super) stop_fails: bool,
    pub(super) stop_hangs: Arc<AtomicBool>,
    pub(super) fail_setup: Option<String>,
    pub(super) fail_close: Arc<AtomicBool>,
    pub(super) entered: Arc<Notify>,
    pub(super) release: Arc<Notify>,
}
impl Driver for FakeDriver {
    fn end_cause(&self) -> Option<IncompleteReason> {
        *self.cause.lock().unwrap()
    }
    fn ended(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        let mut ended = self.ended.clone();
        Box::pin(async move {
            let _ = ended.changed().await;
        })
    }
    fn session_window(&self) -> Option<crate::compositor::WindowAddress> {
        Some(crate::compositor::WindowAddress("0xabc".into()))
    }
    fn list_tools(&mut self) -> DriverFuture<'_, Vec<Tool>> {
        Box::pin(async { Ok(vec![]) })
    }
    fn call_tool<'a>(&'a mut self, tool: &'a str, arguments: Value) -> DriverFuture<'a, Value> {
        Box::pin(async move {
            self.calls
                .lock()
                .unwrap()
                .push((tool.to_owned(), arguments.clone()));
            if tool == "list_windows" {
                return match *self.windows.lock().unwrap() {
                    WindowsReply::Listed => Ok(
                        json!({"content":[{"type":"text","text":"window_id=7 pid=158 \"Sign in made-up-value\"\nwindow_id=9 pid=212 \"File Upload\""}]}),
                    ),
                    WindowsReply::Failed => Err(DriverError::Call {
                        tool: tool.into(),
                        reason: "fixture failure".into(),
                    }),
                    WindowsReply::ReportedFailure => {
                        Ok(json!({"isError":true,"content":[{"type":"text","text":"list failed"}]}))
                    }
                };
            }
            if let Some(text) = arguments["refuse_with"].as_str() {
                return Ok(
                    json!({"isError":arguments["non_error"] != true,"content":[{"type":"text","text":text}]}),
                );
            }
            if tool == "get_window_state" {
                return Ok(self.window_state.lock().unwrap().clone());
            }
            if arguments["element_token"] == REFUSED_TOKEN {
                return Ok(
                    json!({"isError":true,"content":[{"type":"text","text":"fixture refusal"}]}),
                );
            }
            if tool == "get_desktop_state" {
                let mode = *self.screenshot.lock().unwrap();
                match mode {
                    Screenshot::Ready => {}
                    Screenshot::Image(mime_type, data) => {
                        return Ok(
                            json!({"content":[{"type":"image","data":data,"mimeType":mime_type}]}),
                        );
                    }
                    Screenshot::NoImage => {
                        return Ok(json!({"content":[{"type":"text","text":"no screenshot"}]}));
                    }
                    Screenshot::ReportedFailure => {
                        return Ok(
                            json!({"isError": true, "content": [{"type":"text", "text":"capture rejected"}]}),
                        );
                    }
                    Screenshot::Failed => {
                        return Err(DriverError::Call {
                            tool: tool.into(),
                            reason: "fixture failure".into(),
                        });
                    }
                    Screenshot::Held => {
                        self.entered.notify_one();
                        self.release.notified().await;
                    }
                }
                return Ok(
                    json!({"content":[{"type":"text","text":"made-up-value"},{"type":"image","data":"AA==","mimeType":"image/png"}]}),
                );
            }
            if arguments["echo_error"] == true {
                return Err(DriverError::Close(arguments.to_string()));
            }
            if arguments["echo"] == true {
                return Ok(
                    json!({"content":[{"type":"text","text":arguments.to_string()}],"structuredContent":arguments}),
                );
            }
            if tool == "slow"
                || (tool == "stop_recording" && self.stop_hangs.load(Ordering::SeqCst))
            {
                self.entered.notify_one();
                self.release.notified().await;
                self.calls
                    .lock()
                    .unwrap()
                    .push(("driver finished".into(), json!({})));
            }
            let duration = match tool {
                "set_config" | "set_agent_cursor_motion" => 200,
                "start_recording" | "read" => 50,
                "run_steps" => ACTION_DURATION_MS,
                _ => 0,
            };
            self.clock.elapsed.fetch_add(duration, Ordering::SeqCst);
            if self.fail_setup.as_deref() == Some(tool) {
                return Ok(json!({"isError":true}));
            }
            if tool == "stop_recording" && self.stop_fails {
                return Ok(
                    json!({"isError":true,"content":[{"type":"text","text":"fixture recording failure"}]}),
                );
            }
            Ok(json!({"driver":self.id}))
        })
    }
    fn close(&mut self) -> DriverFuture<'_, ()> {
        Box::pin(async {
            self.calls
                .lock()
                .unwrap()
                .push(("driver close".into(), json!({})));
            *self.cause.lock().unwrap() = None;
            self.closed.store(true, Ordering::SeqCst);
            if self.fail_close.load(Ordering::SeqCst) {
                return Err(DriverError::Close("fixture close failure".into()));
            }
            Ok(())
        })
    }
}
