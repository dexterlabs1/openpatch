impl OpenPatch {
    fn listen_command(&mut self, name: &str, mut args: Value) -> bool {
        if args.get("expected_revision").is_none() {
            args["expected_revision"] = json!(self.revision);
        }
        // Capture and observation are performance operations. In particular,
        // polling must not publish layout, create history or rebuild the graph.
        match execute_shared(&self.shared, name, args.clone()) {
            Ok(receipt) => {
                self.rack.accept_listen_receipt(name, receipt);
                if name == "start_listen_capture" {
                    self.status = "Listen capture requested · awaiting audio".into();
                } else if name == "cancel_listen_capture" {
                    self.status = "Listen cancellation requested · awaiting worker".into();
                }
                true
            }
            Err(error) => {
                self.rack.listen_command_error(name, &args, error.clone());
                if name != "inspect_listen" {
                    self.status = error;
                }
                false
            }
        }
    }

    fn listen_capture_ui(&mut self, ctx: &egui::Context) {
        for (name, args) in self.rack.listen_poll_commands(&self.patch, self.revision) {
            self.listen_command(name, args);
        }
        if self
            .patch
            .nodes
            .iter()
            .any(|node| node.kind == "rackmcp_listen")
            || self.rack.listen_capture_pending()
        {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if let Some(RackAction::Command(name, args)) =
            self.rack.listen_inspector(ctx, self.revision)
        {
            self.listen_command(name, args);
        }
    }
}
