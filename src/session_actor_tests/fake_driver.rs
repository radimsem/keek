use super::*;
#[derive(Clone, Copy)]
pub(super) enum Screenshot {
    Ready,
    Held,
    Failed,
    ReportedFailure,
}
pub(super) struct FakeDriver {
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
            if tool == "get_desktop_state" {
                let mode = *self.screenshot.lock().unwrap();
                match mode {
                    Screenshot::Ready => {}
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
            self.closed.store(true, Ordering::SeqCst);
            if self.fail_close.load(Ordering::SeqCst) {
                return Err(DriverError::Close("fixture close failure".into()));
            }
            Ok(())
        })
    }
}
