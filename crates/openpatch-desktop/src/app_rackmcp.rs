impl OpenPatch {
    fn rackmcp_command(&mut self, name: &str, mut args: Value) -> bool {
        if args.get("expected_revision").is_none() {
            args["expected_revision"] = json!(self.revision);
        }
        // Transport and coherent observation are performance operations. They
        // must not publish layout, create history, or rebuild the callback graph.
        match execute_shared(&self.shared, name, args.clone()) {
            Ok(receipt) => {
                self.rack.accept_rackmcp_receipt(name, receipt);
                if name == "rackmcp_transport" {
                    self.status =
                        "Sequencer transport accepted · awaiting audio observation".into();
                }
                true
            }
            Err(error) => {
                self.rack.rackmcp_command_error(name, &args, error.clone());
                if name != "inspect_rackmcp_sequence" {
                    self.status = error;
                }
                false
            }
        }
    }

    fn rackmcp_live_ui(&mut self, ctx: &egui::Context) {
        for (name, args) in self.rack.rackmcp_poll_commands(&self.patch, self.revision) {
            self.rackmcp_command(name, args);
        }
        if self
            .patch
            .nodes
            .iter()
            .any(|node| node.kind == "rackmcp_sequencer")
        {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
}
