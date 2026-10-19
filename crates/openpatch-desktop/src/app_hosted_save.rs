// Native Save/Save As rendezvous with a live hosted graph across UI frames.
// Capture completion and persistence are separate: only a successful write may
// register the sealed checkpoint as saved.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
struct HostedSave {
    request_id: u64,
    ticket: openpatch::control::HostedProjectCaptureTicket,
    name: String,
    new_file: bool,
    token: Option<FileToken>,
}

impl OpenPatch {
    fn hosted_save_pending(&self) -> bool {
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            self.hosted_save.is_some()
        }
        #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
        {
            false
        }
    }

    fn begin_hosted_save(&mut self, name: &str, new_file: bool) -> Result<bool, String> {
        if self.hosted_save_pending() {
            return Err("A plugin-state save is already in progress".into());
        }
        // Hosted plugin capture exists only where the binary host does (macOS arm64).
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        let needs_capture = self
            .shared
            .try_lock()
            .map_err(|_| "Project is busy; try again".to_string())?
            .requires_hosted_project_capture();
        #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
        let needs_capture = false;
        if !needs_capture {
            return Ok(false);
        }
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            self.rack
                .reconcile_document(&self.patch, &self.project, &self.catalog);
            self.publish_current_layout()?;
            let ticket = self
                .shared
                .try_lock()
                .map_err(|_| "Project is busy; try again".to_string())?
                .begin_hosted_project_capture(Some(self.revision))?;
            let request_id = self
                .audio
                .as_mut()
                .ok_or("Plugin-state save requires the current audio engine")?
                .begin_hosted_capture(ticket.clone())?;
            self.hosted_save = Some(HostedSave {
                request_id,
                ticket,
                name: name.into(),
                new_file,
                token: self.active_token.clone(),
            });
            self.status = "Saving plugin state…".into();
            Ok(true)
        }
        #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
        {
            let _ = (name, new_file);
            Err("Live plugin-state capture currently requires macOS ARM".into())
        }
    }

    fn service_hosted_save(&mut self) {
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            let Some(pending) = self.hosted_save.as_ref() else {
                return;
            };
            let result = match self.audio.as_mut() {
                Some(audio) => audio.take_hosted_capture_result(pending.request_id),
                None => Some(Err(
                    "Audio engine stopped before plugin-state capture completed".into(),
                )),
            };
            let Some(result) = result else {
                return;
            };
            let pending = self.hosted_save.take().unwrap();
            let captured = result
                .and_then(|receipt| {
                    self.shared
                        .try_lock()
                        .map_err(|_| "Project is busy; save again".to_string())?
                        .complete_hosted_project_capture(&pending.ticket, receipt)
                })
                .and_then(|captured| {
                    Ok((
                        Document::from_project(captured.snapshot.document)?,
                        captured.checkpoint,
                    ))
                });
            match captured {
                Ok((document, checkpoint)) => self.write_project_save(
                    &pending.name,
                    pending.new_file,
                    pending.token.as_ref(),
                    document,
                    checkpoint,
                ),
                Err(error) => {
                    self.status =
                        format!("Save failed: {error}. The project was not marked saved.");
                    self.restore_save_continuation();
                }
            }
        }
    }
}
