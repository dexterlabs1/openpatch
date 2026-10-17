impl OpenPatch {
    fn bridge_command(&mut self, name: &str, mut args: Value) -> bool {
        if args.get("expected_revision").is_none() {
            args["expected_revision"] = json!(self.revision);
        }
        if args["expected_revision"].as_u64() != Some(self.revision) {
            self.status = "Bridge action needs the current document revision".into();
            self.rack.bridge_status_error(self.status.clone());
            return false;
        }
        if name == "inspect_bridge" {
            return match execute_shared(&self.shared, name, args) {
                Ok(status) => {
                    self.rack.accept_bridge_status(status);
                    true
                }
                Err(error) => {
                    self.rack.bridge_status_error(error);
                    false
                }
            };
        }
        let result = match name {
            "_native_bridge_start" => {
                self.mcp_session
                    .bridge_action("start", &self.shared, Path::new(&self.workspace))
            }
            "_native_bridge_stop" => {
                self.mcp_session
                    .bridge_action("stop", &self.shared, Path::new(&self.workspace))
            }
            "_native_bridge_restart" => {
                self.mcp_session
                    .bridge_action("restart", &self.shared, Path::new(&self.workspace))
            }
            "_native_bridge_save" => {
                self.save_current();
                Ok(())
            }
            "_native_bridge_show" => {
                self.mcp_session.open();
                Ok(())
            }
            _ => Err("Unknown native Bridge action".to_owned()),
        };
        match result {
            Ok(()) => {
                // Lifecycle acknowledges host actions; the refreshed host status
                // supplies lamps. Drawing never starts a server or alters DSP.
                self.bridge_command("inspect_bridge", json!({"expected_revision":self.revision}));
                true
            }
            Err(error) => {
                self.status = error.clone();
                self.rack.bridge_status_error(error);
                false
            }
        }
    }

    fn bridge_live_ui(&mut self, ctx: &egui::Context) {
        if self.rack.bridge_poll_due(&self.patch, self.revision) {
            self.bridge_command("inspect_bridge", json!({"expected_revision":self.revision}));
        }
        if self
            .patch
            .nodes
            .iter()
            .any(|node| node.kind == "rackmcp_bridge")
        {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
}
