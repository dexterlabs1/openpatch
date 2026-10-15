// Included in performance.rs: the callback owns Processing; control owns arm/ack.
const LEARN_IDLE: u64 = 0;
const LEARN_ARMED: u64 = 1;
const LEARN_PROCESSING: u64 = 2;
const LEARN_READY: u64 = 3;
const LEARN_CANCELLED: u64 = 4;
const LEARN_PHASE_MASK: u64 = 7;
static NEXT_LEARN_EPOCH: AtomicU64 = AtomicU64::new(1);
static NEXT_MIDI_BACKEND: AtomicU64 = AtomicU64::new(1);
use openpatch_core::MidiLearnKind;
fn learn_kind_tag(kind: MidiLearnKind) -> u32 {
    match kind {
        MidiLearnKind::Cc => 1,
        MidiLearnKind::Gate => 2,
    }
}

fn learn_tag(serial: u64, phase: u64) -> u64 {
    (serial << 3) | phase
}
fn learn_phase(tag: u64) -> u64 {
    tag & LEARN_PHASE_MASK
}
fn learn_serial(tag: u64) -> u64 {
    tag >> 3
}
fn learn_epoch() -> u64 {
    NEXT_LEARN_EPOCH.fetch_add(1, Ordering::Relaxed)
}

struct MidiLearnMailbox {
    tag: AtomicU64,
    token: AtomicU64,
    kind: AtomicU32,
    epoch: AtomicU64,
    min_revision: AtomicU64,
    cell: AtomicU32,
    before: [AtomicU32; 16],
    after: [AtomicU32; 16],
    frame: AtomicU64,
    sequence: AtomicU64,
}
impl MidiLearnMailbox {
    fn new() -> Self {
        Self {
            tag: AtomicU64::new(0),
            token: AtomicU64::new(0),
            kind: AtomicU32::new(0),
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
        self.cancel_armed_through(u64::MAX)
    }
    /// A delayed safety action may cancel only tickets accepted before it.
    fn cancel_armed_through(&self, cutoff: u64) -> Result<bool, ()> {
        loop {
            let tag = self.tag.load(Ordering::Acquire);
            match learn_phase(tag) {
                LEARN_PROCESSING | LEARN_READY => return Err(()),
                LEARN_ARMED => {
                    if learn_serial(tag) > cutoff {
                        return Ok(false);
                    }
                    if self
                        .tag
                        .compare_exchange(
                            tag,
                            learn_tag(learn_serial(tag), LEARN_CANCELLED),
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
    fn matches(&self, binding: &MidiLearnBinding, revision: u64) -> bool {
        self.kind.load(Ordering::Acquire) == learn_kind_tag(binding.kind)
            && self.token.load(Ordering::Acquire) == binding.token
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
#[derive(Clone, PartialEq, Eq)]
struct MidiGateSignature {
    assignments: [i16; 16],
    options: [u32; 3],
    bypassed: bool,
    rate: u32,
}
#[derive(Clone, PartialEq, Eq)]
enum MidiLearnSignature {
    Cc(MidiCcSignature),
    Gate(MidiGateSignature),
}
impl MidiLearnSignature {
    fn from_node(node: &openpatch_core::Node, rate: u32) -> Option<Self> {
        let option =
            |name: &str, default: f32| node.params.get(name).copied().unwrap_or(default).to_bits();
        match SavedModuleState::from_node(node)? {
            SavedModuleState::MidiCc {
                assignments,
                values,
            } => Some(Self::Cc(MidiCcSignature {
                assignments,
                values,
                options: [
                    option("smooth", 1.0),
                    option("mpe", 0.0),
                    option("high_resolution", 0.0),
                    option("channel", 0.0),
                ],
                bypassed: node.bypassed,
                rate,
            })),
            SavedModuleState::MidiGateMappings(assignments) => {
                Some(Self::Gate(MidiGateSignature {
                    assignments,
                    options: [
                        option("velocity", 0.0),
                        option("mpe", 0.0),
                        option("channel", 0.0),
                    ],
                    bypassed: node.bypassed,
                    rate,
                }))
            }
            _ => None,
        }
    }
    fn kind(&self) -> MidiLearnKind {
        match self {
            Self::Cc(_) => MidiLearnKind::Cc,
            Self::Gate(_) => MidiLearnKind::Gate,
        }
    }
    fn set_assignments(&mut self, maps: [i16; 16]) {
        match self {
            Self::Cc(s) => s.assignments = maps,
            Self::Gate(s) => s.assignments = maps,
        }
    }
}
struct MidiLearnControl {
    slot: usize,
    token: u64,
    epoch: u64,
    signature: MidiLearnSignature,
}
struct MidiLearnBinding {
    slot: usize,
    token: u64,
    epoch: u64,
    kind: MidiLearnKind,
    handle: ModuleHandle,
    processing: u64,
}

impl Performance {
    fn sync_midi_learns(&self, control: &mut Control, patch: &Patch, revision: u64) {
        // Session resolves Ready before retirement. Unexpected direct callers can
        // retire Armed but cannot erase a completion reserved by the callback.
        control.midi_learns.retain(|id, cc| {
            let retained = control.entries.get(id).is_some_and(|entry| {
                entry.kind.learn_kind() == Some(cc.signature.kind()) && entry.token == cc.token
            });
            if !retained {
                let _ = self.shared.midi_learns[cc.slot].cancel_armed();
            }
            retained
                || matches!(
                    learn_phase(self.shared.midi_learns[cc.slot].tag.load(Ordering::Acquire)),
                    LEARN_PROCESSING | LEARN_READY
                )
        });
        for node in patch
            .nodes
            .iter()
            .filter(|node| MidiLearnKind::from_module_kind(&node.kind).is_some())
        {
            let Some(entry) = control.entries.get(&node.id) else {
                continue;
            };
            let Some(signature) = MidiLearnSignature::from_node(node, patch.sample_rate) else {
                continue;
            };
            if let Some(cc) = control.midi_learns.get_mut(&node.id) {
                let mailbox = &self.shared.midi_learns[cc.slot];
                if cc.token != entry.token {
                    if matches!(
                        learn_phase(mailbox.tag.load(Ordering::Acquire)),
                        LEARN_PROCESSING | LEARN_READY
                    ) {
                        continue;
                    }
                    cc.token = entry.token;
                    cc.slot = entry.slot;
                    cc.epoch = learn_epoch();
                    cc.signature = signature;
                    continue;
                }
                if cc.signature != signature {
                    let tag = mailbox.tag.load(Ordering::Acquire);
                    let mut learned = cc.signature.clone();
                    if learn_phase(tag) == LEARN_READY {
                        learned.set_assignments(mailbox.maps(true));
                    }
                    if learned == signature && learn_phase(tag) == LEARN_READY {
                        // Reconciliation authorizes exactly the audio-owned map;
                        // acknowledgment still owns the durable slot afterward.
                        cc.signature = signature;
                    } else if !matches!(learn_phase(tag), LEARN_PROCESSING | LEARN_READY) {
                        let _ = mailbox.cancel_armed();
                        cc.epoch = learn_epoch();
                        cc.signature = signature;
                    }
                }
            } else {
                let mailbox = &self.shared.midi_learns[entry.slot];
                if matches!(
                    learn_phase(mailbox.tag.load(Ordering::Acquire)),
                    LEARN_PROCESSING | LEARN_READY
                ) {
                    continue;
                }
                control.midi_learns.insert(
                    node.id.clone(),
                    MidiLearnControl {
                        slot: entry.slot,
                        token: entry.token,
                        epoch: learn_epoch(),
                        signature,
                    },
                );
            }
        }
        let _ = revision;
    }
    fn plan_midi_learns(
        &self,
        control: &Control,
        patch: &Patch,
        engine: &Engine,
        plan: &mut ModulePlan,
    ) -> Result<(), String> {
        for node in patch
            .nodes
            .iter()
            .filter(|node| MidiLearnKind::from_module_kind(&node.kind).is_some())
        {
            let Some(cc) = control.midi_learns.get(&node.id) else {
                continue;
            };
            if MidiLearnKind::from_module_kind(&node.kind) != Some(cc.signature.kind()) {
                continue;
            }
            let matching = MidiLearnSignature::from_node(node, patch.sample_rate).as_ref()
                == Some(&cc.signature);
            plan.midi_learns.push(MidiLearnBinding {
                kind: cc.signature.kind(),
                slot: cc.slot,
                token: cc.token,
                epoch: if matching { cc.epoch } else { 0 },
                handle: engine.resolve_module_state(&node.id)?,
                processing: 0,
            });
        }
        Ok(())
    }
    fn arm_midi_learn(&self, kind: MidiLearnKind, id: &str, cell: u8) -> Result<Value, String> {
        if cell >= 16 {
            return Err("MIDI cell is outside 1..16".into());
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
            .filter(|e| e.kind.learn_kind() == Some(kind))
            .ok_or("module is not MIDI")?;
        if entry.bypassed {
            return Err("MIDI Learn is unavailable while bypassed".into());
        }
        let cc = control
            .midi_learns
            .get(id)
            .ok_or("MIDI Learn registry unavailable")?;
        if cc.signature.kind() != kind || cc.token != entry.token {
            return Err("previous MIDI decision awaits reconciliation".into());
        }
        let mailbox = &self.shared.midi_learns[cc.slot];
        let prior = mailbox.tag.load(Ordering::Acquire);
        if matches!(learn_phase(prior), LEARN_PROCESSING | LEARN_READY) {
            return Err("MIDI Learn decision awaits reconciliation".into());
        }
        // Re-arming is an explicit atomic cancellation of the older Armed ticket.
        mailbox
            .cancel_armed()
            .map_err(|_| "MIDI Learn decision awaits reconciliation".to_owned())?;
        let serial = control.next_learn_serial;
        if serial == 0 || serial > (u64::MAX >> 3) {
            return Err("MIDI Learn serial exhausted".into());
        }
        let token = cc.token;
        let epoch = cc.epoch;
        mailbox.kind.store(learn_kind_tag(kind), Ordering::Relaxed);
        mailbox.token.store(token, Ordering::Relaxed);
        mailbox.epoch.store(epoch, Ordering::Relaxed);
        mailbox.cell.store(u32::from(cell), Ordering::Relaxed);
        mailbox
            .min_revision
            .store(control.revision, Ordering::Relaxed);
        mailbox
            .tag
            .store(learn_tag(serial, LEARN_ARMED), Ordering::Release);
        control.next_learn_serial += 1;
        Ok(
            json!({"kind":kind.module_kind(),"backend":self.shared.backend,"instance":token,"epoch":epoch,"serial":serial,"cell":cell+1,"status":"queued"}),
        )
    }
    fn ready_midi_learns(&self) -> Vec<openpatch::control::MidiLearnReceipt> {
        let control = self.control.lock().unwrap_or_else(|e| e.into_inner());
        let mut ready = Vec::new();
        for (id, cc) in &control.midi_learns {
            let mailbox = &self.shared.midi_learns[cc.slot];
            let tag = mailbox.tag.load(Ordering::Acquire);
            if learn_phase(tag) != LEARN_READY
                || mailbox.token.load(Ordering::Relaxed) != cc.token
                || mailbox.epoch.load(Ordering::Relaxed) != cc.epoch
                || mailbox.kind.load(Ordering::Relaxed) != learn_kind_tag(cc.signature.kind())
            {
                continue;
            }
            ready.push(openpatch::control::MidiLearnReceipt {
                kind: cc.signature.kind(),
                backend: self.shared.backend,
                id: id.clone(),
                instance: cc.token,
                epoch: cc.epoch,
                frame: mailbox.frame.load(Ordering::Relaxed),
                sequence: mailbox.sequence.load(Ordering::Relaxed),
                completion: openpatch_core::MidiLearnCompletion {
                    serial: learn_serial(tag),
                    cell: mailbox.cell.load(Ordering::Relaxed) as u8,
                    before: mailbox.maps(false),
                    after: mailbox.maps(true),
                },
            });
        }
        ready.sort_by_key(|r| (r.frame, r.sequence));
        ready
    }
    fn ack_midi_learn(&self, receipt: &openpatch::control::MidiLearnReceipt) -> Result<(), String> {
        let control = self
            .control
            .lock()
            .map_err(|_| "Module event registry poisoned")?;
        let cc = control
            .midi_learns
            .get(&receipt.id)
            .ok_or("obsolete MIDI Learn receipt")?;
        let mailbox = &self.shared.midi_learns[cc.slot];
        let ready = learn_tag(receipt.completion.serial, LEARN_READY);
        if receipt.backend != self.shared.backend
            || receipt.kind != cc.signature.kind()
            || mailbox.kind.load(Ordering::Acquire) != learn_kind_tag(receipt.kind)
            || cc.token != receipt.instance
            || cc.epoch != receipt.epoch
            || mailbox.token.load(Ordering::Acquire) != receipt.instance
            || mailbox.epoch.load(Ordering::Acquire) != receipt.epoch
            || mailbox.cell.load(Ordering::Acquire) != u32::from(receipt.completion.cell)
        {
            return Err("obsolete MIDI Learn receipt".into());
        }
        mailbox
            .tag
            .compare_exchange(
                ready,
                learn_tag(receipt.completion.serial, LEARN_IDLE),
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .map_err(|_| "MIDI Learn receipt is no longer ready")?;
        Ok(())
    }
    fn cancel_learn_kind(&self, kind: MidiLearnKind, id: &str) -> Result<Value, String> {
        self.prepare_learn_edit_kind(&[id.to_owned()], Some(kind))?;
        Ok(json!({"kind":kind.module_kind(),"status":"cancelled"}))
    }
    fn learn_kind_active(&self, kind: MidiLearnKind, uncommitted: bool) -> bool {
        self.shared.midi_learns.iter().any(|m| {
            let tag = m.tag.load(Ordering::Acquire);
            m.kind.load(Ordering::Acquire) == learn_kind_tag(kind)
                && (matches!(learn_phase(tag), LEARN_PROCESSING | LEARN_READY)
                    || (!uncommitted && learn_phase(tag) == LEARN_ARMED))
        })
    }
    fn prepare_learn_edit(&self, ids: &[String]) -> Result<(), String> {
        self.prepare_learn_edit_kind(ids, None)
    }
    fn prepare_learn_edit_kind(
        &self,
        ids: &[String],
        kind: Option<MidiLearnKind>,
    ) -> Result<(), String> {
        let control = self
            .control
            .lock()
            .map_err(|_| "Module event registry poisoned")?;
        for id in ids {
            if let Some(kind) = kind {
                let entry = control
                    .entries
                    .get(id)
                    .ok_or("MIDI Learn module not found")?;
                if entry.kind.learn_kind() != Some(kind) {
                    return Err("MIDI Learn module kind mismatch".into());
                }
            }
            if let Some(cc) = control.midi_learns.get(id) {
                if kind.is_some_and(|kind| cc.signature.kind() != kind) {
                    return Err("MIDI Learn module kind mismatch".into());
                }
                if matches!(
                    learn_phase(self.shared.midi_learns[cc.slot].tag.load(Ordering::Acquire)),
                    LEARN_PROCESSING | LEARN_READY
                ) {
                    return Err("MIDI Learn decision awaits reconciliation; retry edit".into());
                }
            }
        }
        for id in ids {
            if let Some(cc) = control.midi_learns.get(id) {
                self.shared.midi_learns[cc.slot]
                    .cancel_armed()
                    .map_err(|_| "MIDI Learn decision awaits reconciliation".to_owned())?;
            }
        }
        Ok(())
    }
}

impl ModulePlan {
    pub(super) fn cancel_all_midi_learn(&self, engine: &mut Engine) {
        for binding in &self.midi_learns {
            if let Some((serial, _)) = engine.midi_learning(binding.handle, binding.kind) {
                engine.cancel_midi_learn(binding.handle, binding.kind, serial);
            }
            // This is a fading graph. Its copied completion has no authority.
            let _ = engine.take_midi_learn_completion(binding.handle, binding.kind);
        }
    }
}
impl EventReader {
    pub(super) fn sync_midi_learn(
        &self,
        engine: &mut Engine,
        plan: &mut ModulePlan,
        revision: u64,
    ) {
        for binding in &mut plan.midi_learns {
            let mailbox = &self.shared.midi_learns[binding.slot];
            let tag = mailbox.tag.load(Ordering::Acquire);
            let valid = !self.shared.closed.load(Ordering::Acquire)
                && self.shared.desired[binding.slot].load(Ordering::Acquire) == binding.token
                && mailbox.matches(binding, revision)
                && learn_phase(tag) == LEARN_ARMED;
            if let Some((serial, cell)) = engine.midi_learning(binding.handle, binding.kind) {
                if !valid
                    || serial != learn_serial(tag)
                    || u32::from(cell) != mailbox.cell.load(Ordering::Relaxed)
                {
                    engine.cancel_midi_learn(binding.handle, binding.kind, serial);
                }
            }
            if valid {
                engine.arm_midi_learn(
                    binding.handle,
                    binding.kind,
                    learn_serial(tag),
                    mailbox.cell.load(Ordering::Relaxed) as u8,
                );
            }
        }
    }
    pub(super) fn before_midi_learn(
        &self,
        engine: &mut Engine,
        plan: &mut ModulePlan,
        revision: u64,
    ) {
        self.sync_midi_learn(engine, plan, revision);
        for binding in &mut plan.midi_learns {
            binding.processing = 0;
            let mailbox = &self.shared.midi_learns[binding.slot];
            let tag = mailbox.tag.load(Ordering::Acquire);
            if self.shared.closed.load(Ordering::Acquire)
                || self.shared.desired[binding.slot].load(Ordering::Acquire) != binding.token
                || !mailbox.matches(binding, revision)
                || learn_phase(tag) != LEARN_ARMED
            {
                if let Some((serial, _)) = engine.midi_learning(binding.handle, binding.kind) {
                    engine.cancel_midi_learn(binding.handle, binding.kind, serial);
                }
                continue;
            }
            if mailbox
                .tag
                .compare_exchange(
                    tag,
                    learn_tag(learn_serial(tag), LEARN_PROCESSING),
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                binding.processing = learn_tag(learn_serial(tag), LEARN_PROCESSING);
                // Recheck the core after claiming: a control cancellation can no
                // longer win, and every possible completion has a durable slot.
                if !engine.arm_midi_learn(
                    binding.handle,
                    binding.kind,
                    learn_serial(tag),
                    mailbox.cell.load(Ordering::Relaxed) as u8,
                ) {
                    mailbox.tag.store(
                        learn_tag(learn_serial(tag), LEARN_CANCELLED),
                        Ordering::Release,
                    );
                    binding.processing = 0;
                }
            } else if let Some((serial, _)) = engine.midi_learning(binding.handle, binding.kind) {
                engine.cancel_midi_learn(binding.handle, binding.kind, serial);
            }
        }
    }
    pub(super) fn after_midi_learn(
        &self,
        engine: &mut Engine,
        plan: &mut ModulePlan,
        frame: u64,
        is_panic: bool,
    ) {
        if is_panic {
            // Future-revision tickets may not have a binding in this graph yet.
            for mailbox in self.shared.midi_learns.iter() {
                let _ = mailbox.cancel_armed();
            }
        }
        for binding in &mut plan.midi_learns {
            let processing = std::mem::take(&mut binding.processing);
            if processing == 0 {
                continue;
            }
            let mailbox = &self.shared.midi_learns[binding.slot];
            if let Some(completion) =
                engine.take_midi_learn_completion(binding.handle, binding.kind)
            {
                if completion.serial == learn_serial(processing)
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
                            .midi_completion_sequence
                            .fetch_add(1, Ordering::Relaxed)
                            + 1,
                        Ordering::Relaxed,
                    );
                    mailbox
                        .tag
                        .store(learn_tag(completion.serial, LEARN_READY), Ordering::Release);
                    continue;
                }
            }
            let cancel = is_panic
                || self.shared.closed.load(Ordering::Acquire)
                || self.shared.desired[binding.slot].load(Ordering::Acquire) != binding.token;
            if cancel {
                engine.cancel_midi_learn(binding.handle, binding.kind, learn_serial(processing));
            }
            mailbox.tag.store(
                learn_tag(
                    learn_serial(processing),
                    if cancel { LEARN_CANCELLED } else { LEARN_ARMED },
                ),
                Ordering::Release,
            );
        }
    }
    pub(super) fn cancel_all_midi_learn(&self, engine: &mut Engine, plan: &mut ModulePlan) {
        // Reset applies to queued/future incarnations as well as this graph.
        for mailbox in self.shared.midi_learns.iter() {
            let _ = mailbox.cancel_armed();
        }
        for binding in &mut plan.midi_learns {
            if let Some((serial, _)) = engine.midi_learning(binding.handle, binding.kind) {
                engine.cancel_midi_learn(binding.handle, binding.kind, serial);
            }
        }
    }
}
