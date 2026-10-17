// Original internal UI release ticket; never retarget a reused textual ID.
// The retained backend and both identity epochs are captured at enqueue time.
pub(crate) struct ManualReleaseTicket {
    backend: Arc<Performance>,
    id: String,
    token: u64,
    action_epoch: u64,
    release: ModuleAction,
    sampled_sequence: Option<u64>,
    deadline: u64,
    release_posted: bool,
    release_sequence: Option<u64>,
    release_frame: u64,
}
struct StockClickRelease {token:u64,action_epoch:u64,frame:u64,sequence:Option<u64>,mask:u16}
impl Drop for ManualReleaseTicket {
    fn drop(&mut self) {self.begin_release();}
}
impl ManualReleaseTicket {
    fn identity_gone(&self) -> bool {
        if self.backend.shared.closed.load(Ordering::Acquire) {return true;}
        let Ok(control)=self.backend.control.lock() else{return true;};
        !control.entries.get(&self.id).is_some_and(|entry|entry.token==self.token && entry.action_epoch==self.action_epoch && !entry.bypassed)
    }
    /// Post exactly one release, retaining this ticket until the source samples
    /// LOW. Drop remains a cancellation path, not an acknowledgment.
    pub(crate) fn begin_release(&mut self) {
        if self.release_posted {return;}
        self.release_frame=self.backend.shared.frame.load(Ordering::Acquire);
        self.release_posted=true;
        if let Ok((receipt,_))=self.backend.enqueue_guarded(&self.id,self.release,Some((self.token,self.action_epoch))) {
            if receipt["emergency_release"]==true {
                self.action_epoch=self.action_epoch.wrapping_add(1).max(1);
            } else {self.release_sequence=receipt["sequence"].as_u64();}
            if self.sampled_sequence.is_some() {
                let mask=stock_click_mask(self.release);
                let mut control=self.backend.control.lock().unwrap_or_else(|e|e.into_inner());
                if control.entries.get(&self.id).is_some_and(|entry|entry.token==self.token && entry.action_epoch==self.action_epoch) {
                    control.stock_click_releases.insert(self.id.clone(),StockClickRelease{token:self.token,action_epoch:self.action_epoch,frame:self.release_frame,sequence:self.release_sequence,mask});
                }
            }
        }
    }
    pub(crate) fn is_finished(&self) -> bool {
        if self.identity_gone() {return true;}
        if !self.release_posted {return false;}
        if self.sampled_sequence.is_none() {return true;}
        if self.release_sequence.is_some_and(|sequence|self.backend.shared.sampled_sequence.load(Ordering::Acquire)<sequence) {return false;}
        let Ok(control)=self.backend.control.lock() else{return true;};
        let Some(entry)=control.entries.get(&self.id).copied().filter(|entry|entry.token==self.token) else{return true;};
        let mask=stock_click_mask(self.release);
        self.backend.shared.meters[entry.slot].read(entry,0).is_some_and(|packet|
            packet.frame>self.release_frame && packet.controls.is_some_and(|controls|controls.pressed_mask & mask==0))
    }
    /// UI-only poll. Release after the source sampled this click, or immediately
    /// if its backend/identity/epoch is gone. Cancellation still uses Drop.
    pub(crate) fn ready_to_release(&self) -> bool {
        let Some(sequence)=self.sampled_sequence else{return true;};
        if self.backend.shared.closed.load(Ordering::Acquire)
            || self.backend.shared.sampled_sequence.load(Ordering::Acquire)>=sequence
            || self.backend.shared.frame.load(Ordering::Acquire)>self.deadline {return true;}
        let Ok(control)=self.backend.control.lock() else{return true;};
        !control.entries.get(&self.id).is_some_and(|entry|entry.token==self.token && entry.action_epoch==self.action_epoch && !entry.bypassed)
    }
}
fn stock_click_mask(release:ModuleAction)->u16 {match release {ModuleAction::ClockRelease=>1,ModuleAction::RunRelease=>2,ModuleAction::ResetRelease=>4,action=>action.seq_gate_button().map_or(0,|(index,_)|1_u16<<(index+3))}}
impl Performance {
    pub(super) fn press_gate_with_ticket(
        self: &Arc<Self>,
        id: &str,
    ) -> Result<ManualReleaseTicket, String> {
        self.press_manual_with_ticket(id, ModuleAction::PushPress, ModuleAction::PushRelease)
    }
    pub(super) fn press_manual_with_ticket(self: &Arc<Self>, id: &str, press: ModuleAction, release: ModuleAction) -> Result<ManualReleaseTicket,String> {
        let valid_pair = matches!((press,release),
            (ModuleAction::PushPress,ModuleAction::PushRelease)
            | (ModuleAction::ClockPress,ModuleAction::ClockRelease)
            | (ModuleAction::RunPress,ModuleAction::RunRelease)
            | (ModuleAction::ResetPress,ModuleAction::ResetRelease))
            || matches!((press.seq_gate_button(),release.seq_gate_button()), (Some((a,true)),Some((b,false))) if a==b);
        if !valid_pair {return Err("Unsupported manual button pair".into());}
        {
            let mut control=self.control.lock().map_err(|_|"Module event registry poisoned")?;
            if let Some(entry)=control.entries.get(id).copied().filter(|entry|entry.kind==ModuleKind::StockSeq3) {
                if let Some(fence)=control.stock_click_releases.get(id).filter(|fence|fence.token==entry.token && fence.action_epoch==entry.action_epoch) {
                    let sampled=fence.sequence.is_none_or(|sequence|self.shared.sampled_sequence.load(Ordering::Acquire)>=sequence)
                        && self.shared.meters[entry.slot].read(entry,0).is_some_and(|packet|packet.frame>fence.frame && packet.controls.is_some_and(|controls|controls.pressed_mask & fence.mask==0));
                    if !sampled {return Err("Previous button release is waiting for the audio engine to sample LOW".into());}
                }
                control.stock_click_releases.remove(id);
            }
        }
        let (receipt,entry)=self.enqueue_guarded(id,press,None)?;
        Ok(ManualReleaseTicket { backend:self.clone(),id:id.into(),token:entry.token,action_epoch:entry.action_epoch,release,
            sampled_sequence:(entry.kind==ModuleKind::StockSeq3).then(||receipt["sequence"].as_u64().expect("accepted press sequence")),
            deadline:self.shared.frame.load(Ordering::Acquire).saturating_add(u64::from(self.rate)*FUTURE_SECONDS),
            release_posted:false,release_sequence:None,release_frame:0,
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
            action: PerformanceAction::Module(action),
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

impl Performance {
    fn enqueue_rackmcp(
        &self,
        id: &str,
        requests: &[RackMcpTransport],
        data: &openpatch_core::rackmcp_sequence::SequenceData,
    ) -> Result<Value, String> {
        if requests.is_empty() || requests.len() > 2 {
            return Err("Expected one action, optionally preceded by a pattern selection".into());
        }
        for (index, request) in requests.iter().enumerate() {
            match request {
                RackMcpTransport::Select { pattern, .. }
                    if usize::from(*pattern) >= data.patterns.len() =>
                {
                    return Err("Pattern index is unavailable".into())
                }
                RackMcpTransport::Seek { bar }
                    if !bar.is_finite() || !(0.0..=100_000.0).contains(bar) =>
                {
                    return Err("Seek bar must be finite and between 0 and 100000".into())
                }
                _ => {}
            }
            if requests.len() == 2
                && ((index == 0 && !matches!(request, RackMcpTransport::Select { .. }))
                    || (index == 1
                        && matches!(request, RackMcpTransport::Select { .. } | RackMcpTransport::Seek { .. })))
            {
                return Err("Expected a pattern selection followed by one transport action".into());
            }
        }
        let fingerprint = openpatch_core::rackmcp_sequence::content_fingerprint(data)?;
        let mut control = self
            .control
            .lock()
            .map_err(|_| "Module event registry poisoned")?;
        if !self.execution_available() {
            self.shared.rejected.fetch_add(1, Ordering::Relaxed);
            return Err("Document execution is unavailable; transport was not queued".into());
        }
        let entry = control
            .entries
            .get(id)
            .copied()
            .ok_or_else(|| format!("Module '{id}' has no live RackMCP sequencer instance"))?;
        if entry.kind != ModuleKind::RackMcpSequencer || entry.bypassed {
            self.shared.rejected.fetch_add(1, Ordering::Relaxed);
            return Err(format!(
                "Module '{id}' is bypassed or is not a RackMCP sequencer"
            ));
        }
        if !control
            .rackmcp_metadata
            .get(id)
            .is_some_and(|(actual, _)| *actual == fingerprint)
        {
            return Err("Sequence content changed; transport was not queued".into());
        }
        let sequence = control.next_sequence;
        control.next_sequence = control.next_sequence.wrapping_add(1).max(1);
        let revision = control.revision;
        let mut fixed = [None; 2];
        for (slot, request) in fixed.iter_mut().zip(requests) {
            *slot = Some(*request);
        }
        let event = Event {
            slot: entry.slot,
            token: entry.token,
            action_epoch: entry.action_epoch,
            cancel_through_serial: 0,
            action: PerformanceAction::RackMcp {
                requests: fixed,
                content_fingerprint: fingerprint,
            },
            sequence,
            revision,
            deadline: self
                .shared
                .frame
                .load(Ordering::Acquire)
                .saturating_add(u64::from(self.rate) * FUTURE_SECONDS),
        };
        self.shared.queued.fetch_add(1, Ordering::Release);
        if control.queue.push(event).is_err() {
            self.shared.queued.fetch_sub(1, Ordering::Release);
            self.shared.backpressure.fetch_add(1, Ordering::Relaxed);
            return Err("Module action queue is full; transport was not queued".into());
        }
        Ok(
            json!({"queued":true,"applied":false,"sequence":sequence,"id":id,"minimum_revision":revision,"future_timeout_seconds":FUTURE_SECONDS}),
        )
    }
}
