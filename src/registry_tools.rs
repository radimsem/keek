use super::*;

impl Registry {
    pub(super) fn list_tools(&mut self, reply: oneshot::Sender<Response>) {
        let mut catalog = self.catalog.clone();
        self.background.spawn(async move {
            let result = wait_catalog(&mut catalog).await;
            let tools = result.map_or_else(|_| vec![], |c| c.tools());
            let _ = reply.send(Response::Tools(serde_json::json!({"tools":tools})));
        });
    }
}
