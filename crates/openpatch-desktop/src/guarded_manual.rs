// Original internal UI release ticket; never retarget a reused textual ID.
// The retained backend and both identity epochs are captured at enqueue time.
pub(crate) struct ManualReleaseTicket {
    backend: Arc<Performance>,
    id: String,
    token: u64,
    action_epoch: u64,
}
impl Drop for ManualReleaseTicket {
    fn drop(&mut self) {
        // Obsolete/closed receivers need no release. Queue-full releases use the
        // existing emergency path only after the same identity guard passes.
        let _ = self.backend.enqueue_guarded(
            &self.id,
            ModuleAction::PushRelease,
            Some((self.token, self.action_epoch)),
        );
    }
}
impl Performance {
    pub(super) fn press_gate_with_ticket(
        self: &Arc<Self>,
        id: &str,
    ) -> Result<ManualReleaseTicket, String> {
        let (_, entry) = self.enqueue_guarded(id, ModuleAction::PushPress, None)?;
        Ok(ManualReleaseTicket {
            backend: self.clone(),
            id: id.into(),
            token: entry.token,
            action_epoch: entry.action_epoch,
        })
    }
    fn enqueue_guarded(
        &self,
        id: &str,
        action: ModuleAction,
        guard: Option<(u64, u64)>,
    ) -> Result<(Value, Entry), String> {
        let mut control = self
            .control
            .lock()
            .map_err(|_| "Module event registry poisoned")?;
        if self.shared.closed.load(Ordering::Acquire) {
            self.shared.rejected.fetch_add(1, Ordering::Relaxed);
            return Err("Audio device closed; module action was not queued".into());
        }
        let Some(entry) = control.entries.get(id).copied() else {
            self.shared.rejected.fetch_add(1, Ordering::Relaxed);
            return Err(format!("Module '{id}' has no live sequencer instance"));
        };
        if guard.is_some_and(|(token, epoch)| token != entry.token || epoch != entry.action_epoch) {
            return Err("Manual release ticket is obsolete".into());
        }
        if !entry.kind.accepts(action) {
            self.shared.rejected.fetch_add(1, Ordering::Relaxed);
            return Err(format!(
                "Module '{id}' ({}) has read-only telemetry or does not support this module action",
                entry.kind.name()
            ));
        }
        if entry.bypassed && !is_release(action) {
            self.shared.rejected.fetch_add(1, Ordering::Relaxed);
            return Err(format!(
                "Module '{id}' is bypassed; module action was not queued"
            ));
        }
        let sequence = control.next_sequence;
        control.next_sequence = control.next_sequence.wrapping_add(1).max(1);
        let revision = control.revision;
        let cancel_through_serial = control.next_learn_serial.saturating_sub(1);
        let event = Event {
            slot: entry.slot,
            token: entry.token,
            action_epoch: entry.action_epoch,
            cancel_through_serial,
            action,
            sequence,
            revision,
            deadline: self
                .shared
                .frame
                .load(Ordering::Acquire)
                .saturating_add(u64::from(self.rate) * FUTURE_SECONDS),
        };
        // Count acceptance before publishing so the consumer cannot report an
        // applied event before its queued counter. A rejected push rolls back.
        self.shared.queued.fetch_add(1, Ordering::Release);
        if control.queue.push(event).is_err() {
            self.shared.queued.fetch_sub(1, Ordering::Release);
            self.shared.backpressure.fetch_add(1, Ordering::Relaxed);
            if is_release(action) {
                self.request_release(control.entries.get_mut(id).unwrap(), cancel_through_serial);
                return Ok((
                    json!({"queued":false,"applied":false,"safety_pending":true,"emergency_release":true,"sequence":sequence,
                    "id":id,"action":action,"minimum_revision":revision}),
                    entry,
                ));
            }
            return Err("Module action queue is full; action was not queued".into());
        }
        Ok((
            json!({"queued":true,"applied":false,"sequence":sequence,"id":id,"action":action,"minimum_revision":revision,"future_timeout_seconds":FUTURE_SECONDS}),
            entry,
        ))
    }
}
