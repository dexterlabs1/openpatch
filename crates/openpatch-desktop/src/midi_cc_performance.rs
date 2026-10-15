// Included in performance.rs: the callback owns Processing; control owns arm/ack.
const CC_IDLE: u64 = 0;
const CC_ARMED: u64 = 1;
const CC_PROCESSING: u64 = 2;
const CC_READY: u64 = 3;
const CC_CANCELLED: u64 = 4;
const CC_PHASE_MASK: u64 = 7;
static NEXT_CC_EPOCH: AtomicU64 = AtomicU64::new(1);

fn cc_tag(serial: u64, phase: u64) -> u64 {
    (serial << 3) | phase
}
fn cc_phase(tag: u64) -> u64 {
    tag & CC_PHASE_MASK
}
fn cc_serial(tag: u64) -> u64 {
    tag >> 3
}
fn cc_epoch() -> u64 {
    NEXT_CC_EPOCH.fetch_add(1, Ordering::Relaxed)
}

struct MidiCcLearnMailbox {
    tag: AtomicU64,
    token: AtomicU64,
    epoch: AtomicU64,
    min_revision: AtomicU64,
    cell: AtomicU32,
    before: [AtomicU32; 16],
    after: [AtomicU32; 16],
    frame: AtomicU64,
    sequence: AtomicU64,
}
impl MidiCcLearnMailbox {
    fn new() -> Self {
        Self {
            tag: AtomicU64::new(0),
            token: AtomicU64::new(0),
            epoch: AtomicU64::new(0),
            min_revision: AtomicU64::new(0),
            cell: AtomicU32::new(0),
            before: std::array::from_fn(|_| AtomicU32::new(0)),
            after: std::array::from_fn(|_| AtomicU32::new(0)),
            frame: AtomicU64::new(0),
            sequence: AtomicU64::new(0),
        }
    }
    fn cancel_armed(&self) -> Result<bool, ()> {
        loop {
            let tag = self.tag.load(Ordering::Acquire);
            match cc_phase(tag) {
                CC_PROCESSING | CC_READY => return Err(()),
                CC_ARMED => {
                    if self
                        .tag
                        .compare_exchange(
                            tag,
                            cc_tag(cc_serial(tag), CC_CANCELLED),
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        )
                        .is_ok()
                    {
                        return Ok(true);
                    }
                }
                _ => return Ok(false),
            }
        }
    }
    fn matches(&self, binding: &MidiCcBinding, revision: u64) -> bool {
        self.token.load(Ordering::Acquire) == binding.token
            && self.epoch.load(Ordering::Acquire) == binding.epoch
            && revision >= self.min_revision.load(Ordering::Acquire)
    }
    fn maps(&self, after: bool) -> [i16; 16] {
        let source = if after { &self.after } else { &self.before };
        std::array::from_fn(|i| source[i].load(Ordering::Relaxed) as u16 as i16)
    }
}

#[derive(Clone, PartialEq, Eq)]
struct MidiCcSignature {
    assignments: [i16; 16],
    values: [u8; 128],
    options: [u32; 4],
    bypassed: bool,
    rate: u32,
}
impl MidiCcSignature {
    fn from_node(node: &openpatch_core::Node, rate: u32) -> Option<Self> {
        let SavedModuleState::MidiCc {
            assignments,
            values,
        } = SavedModuleState::from_node(node)?
        else {
            return None;
        };
        Some(Self {
            assignments,
            values,
            options: [
                node.params.get("smooth").copied().unwrap_or(1.0).to_bits(),
                node.params.get("mpe").copied().unwrap_or(0.0).to_bits(),
                node.params
                    .get("high_resolution")
                    .copied()
                    .unwrap_or(0.0)
                    .to_bits(),
                node.params.get("channel").copied().unwrap_or(0.0).to_bits(),
            ],
            bypassed: node.bypassed,
            rate,
        })
    }
}
struct MidiCcLearnControl {
    slot: usize,
    token: u64,
    epoch: u64,
    signature: MidiCcSignature,
}
struct MidiCcBinding {
    slot: usize,
    token: u64,
    epoch: u64,
    handle: ModuleHandle,
    processing: u64,
}

impl Performance {
    fn sync_cc_learns(&self, control: &mut Control, patch: &Patch, revision: u64) {
        // Session resolves Ready before retirement. Unexpected direct callers can
        // retire Armed but cannot erase a completion reserved by the callback.
        control.cc_learns.retain(|id, cc| {
            let retained = control
                .entries
                .get(id)
                .is_some_and(|entry| entry.kind == ModuleKind::MidiCc && entry.token == cc.token);
            if !retained {
                let _ = self.shared.cc_learns[cc.slot].cancel_armed();
            }
            retained
                || matches!(
                    cc_phase(self.shared.cc_learns[cc.slot].tag.load(Ordering::Acquire)),
                    CC_PROCESSING | CC_READY
                )
        });
        for node in patch.nodes.iter().filter(|node| node.kind == "midi_cc") {
            let Some(entry) = control.entries.get(&node.id) else {
                continue;
            };
            let Some(signature) = MidiCcSignature::from_node(node, patch.sample_rate) else {
                continue;
            };
            if let Some(cc) = control.cc_learns.get_mut(&node.id) {
                let mailbox = &self.shared.cc_learns[cc.slot];
                if cc.token != entry.token {
                    if matches!(
                        cc_phase(mailbox.tag.load(Ordering::Acquire)),
                        CC_PROCESSING | CC_READY
                    ) {
                        continue;
                    }
                    cc.token = entry.token;
                    cc.slot = entry.slot;
                    cc.epoch = cc_epoch();
                    cc.signature = signature;
                    continue;
                }
                if cc.signature != signature {
                    let tag = mailbox.tag.load(Ordering::Acquire);
                    let mut learned = cc.signature.clone();
                    if cc_phase(tag) == CC_READY {
                        learned.assignments = mailbox.maps(true);
                    }
                    if learned == signature && cc_phase(tag) == CC_READY {
                        // Reconciliation authorizes exactly the audio-owned map;
                        // acknowledgment still owns the durable slot afterward.
                        cc.signature = signature;
                    } else if !matches!(cc_phase(tag), CC_PROCESSING | CC_READY) {
                        let _ = mailbox.cancel_armed();
                        cc.epoch = cc_epoch();
                        cc.signature = signature;
                    }
                }
            } else {
                let mailbox = &self.shared.cc_learns[entry.slot];
                if matches!(
                    cc_phase(mailbox.tag.load(Ordering::Acquire)),
                    CC_PROCESSING | CC_READY
                ) {
                    continue;
                }
                control.cc_learns.insert(
                    node.id.clone(),
                    MidiCcLearnControl {
                        slot: entry.slot,
                        token: entry.token,
                        epoch: cc_epoch(),
                        signature,
                    },
                );
            }
        }
        let _ = revision;
    }
    fn plan_cc_learns(
        &self,
        control: &Control,
        patch: &Patch,
        engine: &Engine,
        plan: &mut ModulePlan,
    ) -> Result<(), String> {
        for node in patch.nodes.iter().filter(|node| node.kind == "midi_cc") {
            let Some(cc) = control.cc_learns.get(&node.id) else {
                continue;
            };
            let matching =
                MidiCcSignature::from_node(node, patch.sample_rate).as_ref() == Some(&cc.signature);
            plan.cc_learns.push(MidiCcBinding {
                slot: cc.slot,
                token: cc.token,
                epoch: if matching { cc.epoch } else { 0 },
                handle: engine.resolve_module_state(&node.id)?,
                processing: 0,
            });
        }
        Ok(())
    }
    fn arm_cc_learn(&self, id: &str, cell: u8) -> Result<Value, String> {
        if cell >= 16 {
            return Err("MIDI CC cell is outside 1..16".into());
        }
        let mut control = self
            .control
            .lock()
            .map_err(|_| "Module event registry poisoned")?;
        if self.shared.closed.load(Ordering::Acquire) {
            return Err("audio backend is closed".into());
        }
        let entry = *control
            .entries
            .get(id)
            .filter(|e| e.kind == ModuleKind::MidiCc)
            .ok_or("module is not MIDI CC")?;
        if entry.bypassed {
            return Err("MIDI CC Learn is unavailable while bypassed".into());
        }
        let cc = control
            .cc_learns
            .get(id)
            .ok_or("MIDI CC Learn registry unavailable")?;
        if cc.token != entry.token {
            return Err("previous MIDI CC decision awaits reconciliation".into());
        }
        let mailbox = &self.shared.cc_learns[cc.slot];
        let prior = mailbox.tag.load(Ordering::Acquire);
        if matches!(cc_phase(prior), CC_PROCESSING | CC_READY) {
            return Err("MIDI CC Learn decision awaits reconciliation".into());
        }
        // Re-arming is an explicit atomic cancellation of the older Armed ticket.
        mailbox
            .cancel_armed()
            .map_err(|_| "MIDI CC Learn decision awaits reconciliation".to_owned())?;
        let serial = control.next_learn_serial;
        if serial == 0 || serial > (u64::MAX >> 3) {
            return Err("MIDI CC Learn serial exhausted".into());
        }
        let token = cc.token;
        let epoch = cc.epoch;
        mailbox.token.store(token, Ordering::Relaxed);
        mailbox.epoch.store(epoch, Ordering::Relaxed);
        mailbox.cell.store(u32::from(cell), Ordering::Relaxed);
        mailbox
            .min_revision
            .store(control.revision, Ordering::Relaxed);
        mailbox
            .tag
            .store(cc_tag(serial, CC_ARMED), Ordering::Release);
        control.next_learn_serial += 1;
        Ok(json!({"instance":token,"epoch":epoch,"serial":serial,"cell":cell+1,"status":"queued"}))
    }
    fn ready_cc_learns(&self) -> Vec<openpatch::control::MidiCcLearnReceipt> {
        let control = self.control.lock().unwrap_or_else(|e| e.into_inner());
        let mut ready = Vec::new();
        for (id, cc) in &control.cc_learns {
            let mailbox = &self.shared.cc_learns[cc.slot];
            let tag = mailbox.tag.load(Ordering::Acquire);
            if cc_phase(tag) != CC_READY
                || mailbox.token.load(Ordering::Relaxed) != cc.token
                || mailbox.epoch.load(Ordering::Relaxed) != cc.epoch
            {
                continue;
            }
            ready.push(openpatch::control::MidiCcLearnReceipt {
                id: id.clone(),
                instance: cc.token,
                epoch: cc.epoch,
                frame: mailbox.frame.load(Ordering::Relaxed),
                sequence: mailbox.sequence.load(Ordering::Relaxed),
                completion: openpatch_core::MidiCcLearnCompletion {
                    serial: cc_serial(tag),
                    cell: mailbox.cell.load(Ordering::Relaxed) as u8,
                    before: mailbox.maps(false),
                    after: mailbox.maps(true),
                },
            });
        }
        ready.sort_by_key(|r| (r.frame, r.sequence));
        ready
    }
    fn ack_cc_learn(&self, receipt: &openpatch::control::MidiCcLearnReceipt) -> Result<(), String> {
        let control = self
            .control
            .lock()
            .map_err(|_| "Module event registry poisoned")?;
        let cc = control
            .cc_learns
            .get(&receipt.id)
            .ok_or("obsolete MIDI CC Learn receipt")?;
        let mailbox = &self.shared.cc_learns[cc.slot];
        let ready = cc_tag(receipt.completion.serial, CC_READY);
        if cc.token != receipt.instance
            || cc.epoch != receipt.epoch
            || mailbox.token.load(Ordering::Acquire) != receipt.instance
            || mailbox.epoch.load(Ordering::Acquire) != receipt.epoch
            || mailbox.cell.load(Ordering::Acquire) != u32::from(receipt.completion.cell)
        {
            return Err("obsolete MIDI CC Learn receipt".into());
        }
        mailbox
            .tag
            .compare_exchange(
                ready,
                cc_tag(receipt.completion.serial, CC_IDLE),
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .map_err(|_| "MIDI CC Learn receipt is no longer ready")?;
        Ok(())
    }
    fn prepare_cc_edit(&self, ids: &[String]) -> Result<(), String> {
        let control = self
            .control
            .lock()
            .map_err(|_| "Module event registry poisoned")?;
        // Inspect every affected mailbox before cancelling any Armed request.
        for id in ids {
            if let Some(cc) = control.cc_learns.get(id) {
                let phase = cc_phase(self.shared.cc_learns[cc.slot].tag.load(Ordering::Acquire));
                if matches!(phase, CC_PROCESSING | CC_READY) {
                    return Err("MIDI CC Learn decision awaits reconciliation; retry edit".into());
                }
            }
        }
        for id in ids {
            if let Some(cc) = control.cc_learns.get(id) {
                self.shared.cc_learns[cc.slot]
                    .cancel_armed()
                    .map_err(|_| "MIDI CC Learn decision awaits reconciliation".to_owned())?;
            }
        }
        Ok(())
    }
}

impl ModulePlan {
    pub(super) fn cancel_midi_cc_learn(&self, engine: &mut Engine) {
        for binding in &self.cc_learns {
            if let Some((serial, _)) = engine.midi_cc_learning(binding.handle) {
                engine.cancel_midi_cc_learn(binding.handle, serial);
            }
            // This is a fading graph. Its copied completion has no authority.
            let _ = engine.take_midi_cc_learn_completion(binding.handle);
        }
    }
}
impl EventReader {
    pub(super) fn sync_midi_cc(&self, engine: &mut Engine, plan: &mut ModulePlan, revision: u64) {
        for binding in &mut plan.cc_learns {
            let mailbox = &self.shared.cc_learns[binding.slot];
            let tag = mailbox.tag.load(Ordering::Acquire);
            let valid = !self.shared.closed.load(Ordering::Acquire)
                && self.shared.desired[binding.slot].load(Ordering::Acquire) == binding.token
                && mailbox.matches(binding, revision)
                && cc_phase(tag) == CC_ARMED;
            if let Some((serial, cell)) = engine.midi_cc_learning(binding.handle) {
                if !valid
                    || serial != cc_serial(tag)
                    || u32::from(cell) != mailbox.cell.load(Ordering::Relaxed)
                {
                    engine.cancel_midi_cc_learn(binding.handle, serial);
                }
            }
            if valid {
                engine.arm_midi_cc_learn(
                    binding.handle,
                    cc_serial(tag),
                    mailbox.cell.load(Ordering::Relaxed) as u8,
                );
            }
        }
    }
    pub(super) fn before_midi_cc(&self, engine: &mut Engine, plan: &mut ModulePlan, revision: u64) {
        self.sync_midi_cc(engine, plan, revision);
        for binding in &mut plan.cc_learns {
            binding.processing = 0;
            let mailbox = &self.shared.cc_learns[binding.slot];
            let tag = mailbox.tag.load(Ordering::Acquire);
            if self.shared.closed.load(Ordering::Acquire)
                || self.shared.desired[binding.slot].load(Ordering::Acquire) != binding.token
                || !mailbox.matches(binding, revision)
                || cc_phase(tag) != CC_ARMED
            {
                if let Some((serial, _)) = engine.midi_cc_learning(binding.handle) {
                    engine.cancel_midi_cc_learn(binding.handle, serial);
                }
                continue;
            }
            if mailbox
                .tag
                .compare_exchange(
                    tag,
                    cc_tag(cc_serial(tag), CC_PROCESSING),
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                binding.processing = cc_tag(cc_serial(tag), CC_PROCESSING);
                // Recheck the core after claiming: a control cancellation can no
                // longer win, and every possible completion has a durable slot.
                if !engine.arm_midi_cc_learn(
                    binding.handle,
                    cc_serial(tag),
                    mailbox.cell.load(Ordering::Relaxed) as u8,
                ) {
                    mailbox
                        .tag
                        .store(cc_tag(cc_serial(tag), CC_CANCELLED), Ordering::Release);
                    binding.processing = 0;
                }
            } else if let Some((serial, _)) = engine.midi_cc_learning(binding.handle) {
                engine.cancel_midi_cc_learn(binding.handle, serial);
            }
        }
    }
    pub(super) fn after_midi_cc(
        &self,
        engine: &mut Engine,
        plan: &mut ModulePlan,
        frame: u64,
        is_panic: bool,
    ) {
        if is_panic {
            // Future-revision tickets may not have a binding in this graph yet.
            for mailbox in self.shared.cc_learns.iter() {
                let _ = mailbox.cancel_armed();
            }
        }
        for binding in &mut plan.cc_learns {
            let processing = std::mem::take(&mut binding.processing);
            if processing == 0 {
                continue;
            }
            let mailbox = &self.shared.cc_learns[binding.slot];
            if let Some(completion) = engine.take_midi_cc_learn_completion(binding.handle) {
                if completion.serial == cc_serial(processing)
                    && u32::from(completion.cell) == mailbox.cell.load(Ordering::Relaxed)
                {
                    for ((before, after), (old, new)) in mailbox
                        .before
                        .iter()
                        .zip(&mailbox.after)
                        .zip(completion.before.into_iter().zip(completion.after))
                    {
                        before.store(old as u16 as u32, Ordering::Relaxed);
                        after.store(new as u16 as u32, Ordering::Relaxed);
                    }
                    mailbox.frame.store(frame, Ordering::Relaxed);
                    mailbox.sequence.store(
                        self.shared
                            .cc_completion_sequence
                            .fetch_add(1, Ordering::Relaxed)
                            + 1,
                        Ordering::Relaxed,
                    );
                    mailbox
                        .tag
                        .store(cc_tag(completion.serial, CC_READY), Ordering::Release);
                    continue;
                }
            }
            let cancel = is_panic
                || self.shared.closed.load(Ordering::Acquire)
                || self.shared.desired[binding.slot].load(Ordering::Acquire) != binding.token;
            if cancel {
                engine.cancel_midi_cc_learn(binding.handle, cc_serial(processing));
            }
            mailbox.tag.store(
                cc_tag(
                    cc_serial(processing),
                    if cancel { CC_CANCELLED } else { CC_ARMED },
                ),
                Ordering::Release,
            );
        }
    }
    pub(super) fn cancel_midi_cc(&self, engine: &mut Engine, plan: &mut ModulePlan) {
        // Reset applies to queued/future incarnations as well as this graph.
        for mailbox in self.shared.cc_learns.iter() {
            let _ = mailbox.cancel_armed();
        }
        for binding in &mut plan.cc_learns {
            if let Some((serial, _)) = engine.midi_cc_learning(binding.handle) {
                engine.cancel_midi_cc_learn(binding.handle, serial);
            }
        }
    }
}
