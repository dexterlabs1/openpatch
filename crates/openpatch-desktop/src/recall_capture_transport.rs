// Fixed-size callback publication; JSON and authored commits stay off audio.
#[derive(Clone, Copy)]
enum RecallCaptureCommand {
    Begin { slot: u8, revision: u64 },
    Stop,
    Cancel,
}
impl RecallCaptureCommand {
    fn apply(self, engine: &mut Engine, handle: ModuleHandle) -> bool {
        match self {
            Self::Begin { slot, revision } => {
                engine.begin_recall_random_capture(handle, slot, revision)
            }
            Self::Stop => engine.stop_recall_random_capture(handle),
            Self::Cancel => engine.cancel_recall_random_capture(handle),
        }
    }
}
struct RecallCaptureMeter {
    serial: AtomicU64,
    token: AtomicU64,
    revision: AtomicU64,
    frame: AtomicU64,
    words: [AtomicU64; 24],
}
impl RecallCaptureMeter {
    fn new() -> Self {
        Self {
            serial: AtomicU64::new(0),
            token: AtomicU64::new(0),
            revision: AtomicU64::new(0),
            frame: AtomicU64::new(0),
            words: std::array::from_fn(|_| AtomicU64::new(0)),
        }
    }
    fn write(
        &self,
        token: u64,
        revision: u64,
        frame: u64,
        state: Option<openpatch_core::RecallRandomCaptureState>,
        consumed: u64,
        accepted: bool,
    ) {
        use openpatch_core::RecallRandomCaptureStatus as Status;
        let mut words = [0u64; 24];
        words[22] = consumed;
        words[23] = u64::from(accepted);
        if let Some(state) = state {
            words[0] = match state.status {
                Status::Recording => 1,
                Status::Complete => 2,
                Status::Empty => 3,
                Status::Cancelled => 4,
            };
            words[1] = state.capture_id;
            words[2] = state.revision;
            words[3] = u64::from(state.slot);
            words[4] = u64::from(state.scale.base_interval.to_bits());
            words[5] = u64::from(state.scale.num_degrees);
            for (i, degree) in state.scale.degrees.iter().enumerate() {
                words[6 + i] =
                    u64::from(degree.voltage.to_bits()) | (u64::from(degree.weight) << 32);
            }
        }
        self.serial.fetch_add(1, Ordering::SeqCst);
        self.token.store(token, Ordering::SeqCst);
        self.revision.store(revision, Ordering::SeqCst);
        self.frame.store(frame, Ordering::SeqCst);
        for (slot, value) in self.words.iter().zip(words) {
            slot.store(value, Ordering::SeqCst);
        }
        self.serial.fetch_add(1, Ordering::SeqCst);
    }
    fn read(
        &self,
        token: u64,
        revision: u64,
    ) -> Option<(u64, openpatch_core::RecallRandomCaptureState, u64, bool)> {
        use openpatch_core::{
            RecallRandomCaptureState, RecallRandomCaptureStatus as Status,
            RecallRandomCapturedScale, RecallRandomDegree,
        };
        for _ in 0..4 {
            let serial = self.serial.load(Ordering::SeqCst);
            if serial & 1 != 0 {
                continue;
            }
            let actual = self.token.load(Ordering::SeqCst);
            let observed = self.revision.load(Ordering::SeqCst);
            let frame = self.frame.load(Ordering::SeqCst);
            let words = self.words.each_ref().map(|w| w.load(Ordering::SeqCst));
            if self.serial.load(Ordering::SeqCst) != serial {
                continue;
            }
            if actual != token || observed != revision || words[3] >= 6 || words[5] > 16 {
                return None;
            }
            let status = match words[0] {
                1 => Status::Recording,
                2 => Status::Complete,
                3 => Status::Empty,
                4 => Status::Cancelled,
                _ => return None,
            };
            let scale = RecallRandomCapturedScale {
                base_interval: f32::from_bits(words[4] as u32),
                num_degrees: words[5] as u8,
                degrees: std::array::from_fn(|i| RecallRandomDegree {
                    voltage: f32::from_bits(words[6 + i] as u32),
                    weight: (words[6 + i] >> 32) as u8,
                }),
            };
            return Some((
                frame,
                RecallRandomCaptureState {
                    capture_id: words[1],
                    revision: words[2],
                    slot: words[3] as u8,
                    status,
                    scale,
                },
                words[22],
                words[23] != 0,
            ));
        }
        None
    }
}
impl Performance {
    fn recall_capture_command(
        &self,
        id: &str,
        action: &str,
        slot: u8,
        revision: u64,
    ) -> Result<Value, String> {
        let mut control = self
            .control
            .lock()
            .map_err(|_| "Module event registry poisoned")?;
        let entry = control
            .entries
            .get(id)
            .copied()
            .filter(|e| e.kind == ModuleKind::RecallRandom)
            .ok_or("No live Recall Random instance")?;
        if action == "inspect" {
            if !self.execution_available() {
                return Err("Recall Random execution is unavailable".into());
            }
            let (frame, capture, consumed, accepted) = self.shared.recall_meters[entry.slot]
                .read(entry.token, control.revision)
                .ok_or("No current Recall Random capture publication")?;
            if let Some((token, sequence)) = control.recall_pending.get(id) {
                if *token == entry.token {
                    if consumed != *sequence {
                        return Err("Recall Random command is awaiting callback acknowledgment; keep audio running and poll again".into());
                    }
                    if !accepted {
                        return Err("The callback rejected the latest Recall Random capture command; inspect recording state or cancel and begin again".into());
                    }
                }
            }
            return Ok(
                json!({"id":id,"instance":entry.token,"revision":control.revision,"frame":frame,"capture":capture}),
            );
        }
        if revision != control.revision {
            return Err("Recall Random document revision changed".into());
        }
        if !self.execution_available() || entry.bypassed {
            return Err("Recall Random execution is unavailable or bypassed".into());
        }
        let command = match action {
            "begin" if slot < 6 => RecallCaptureCommand::Begin { slot, revision },
            "stop" => RecallCaptureCommand::Stop,
            "cancel" => RecallCaptureCommand::Cancel,
            _ => return Err("Invalid Recall Random capture command".into()),
        };
        let sequence = control.next_sequence;
        control.next_sequence = sequence.wrapping_add(1).max(1);
        let event = Event {
            slot: entry.slot,
            token: entry.token,
            action_epoch: entry.action_epoch,
            cancel_through_serial: 0,
            action: PerformanceAction::Recall(command),
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
            return Err("Module action queue is full".into());
        }
        let live_tokens: std::collections::BTreeSet<_> =
            control.entries.values().map(|entry| entry.token).collect();
        control
            .recall_pending
            .retain(|_, (token, _)| live_tokens.contains(token));
        control
            .recall_pending
            .insert(id.into(), (entry.token, sequence));
        Ok(
            json!({"id":id,"instance":entry.token,"sequence":sequence,"queued":true,"applied":false,"revision":revision}),
        )
    }
}
