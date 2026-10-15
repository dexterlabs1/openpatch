//! Durable audio Learn decisions become serialized mapping-only history.
use super::*;

pub(super) fn mapping_cell(parameter: &str) -> Option<usize> {
    let cell = parameter.strip_prefix("cc")?.parse::<usize>().ok()?;
    ((1..=16).contains(&cell) && parameter == format!("cc{cell}")).then_some(cell - 1)
}

impl Session {
    pub(super) fn midi_cc_ids(&self) -> Vec<String> {
        self.patch
            .nodes
            .iter()
            .filter(|node| node.kind == "midi_cc")
            .map(|node| node.id.clone())
            .collect()
    }

    pub(super) fn check_midi_cc_capture(&self) -> Result<(), String> {
        if self
            .live_module_events
            .as_ref()
            .is_some_and(|backend| backend.midi_cc_learning_uncommitted())
        {
            Err("MIDI CC Learn decision awaits reconciliation; retry capture".into())
        } else {
            Ok(())
        }
    }

    pub(super) fn prepare_midi_cc_edit(&self, ids: &[String]) -> Result<(), String> {
        if !ids.is_empty() {
            if let Some(backend) = &self.live_module_events {
                backend.prepare_midi_cc_edit(ids)?;
            }
        }
        Ok(())
    }

    pub(super) fn midi_cc_conflicts(
        &self,
        candidate: &Patch,
        effects: &EditEffects,
    ) -> Vec<String> {
        self.patch
            .nodes
            .iter()
            .filter(|node| node.kind == "midi_cc")
            .filter(|node| {
                effects.retired.contains(&node.id)
                    || effects.initialized.contains(&node.id)
                    || effects.load_saved.contains_key(&node.id)
                    || candidate
                        .nodes
                        .iter()
                        .find(|new| new.id == node.id)
                        .is_none_or(|new| {
                            new.kind != node.kind
                                || new.bypassed != node.bypassed
                                || new.params != node.params
                        })
            })
            .map(|node| node.id.clone())
            .collect()
    }

    pub(super) fn midi_cc_learn_tool(&mut self, name: &str, args: &Value) -> Result<Value, String> {
        self.check_open()?;
        self.check_revision(expected_revision(args)?)?;
        let id = string(args, "id")?;
        let node = self
            .patch
            .nodes
            .iter()
            .find(|node| node.id == id)
            .ok_or("module not found")?;
        if node.kind != "midi_cc" {
            return Err("MIDI CC Learn requires a midi_cc module".into());
        }
        let backend = self
            .live_module_events
            .as_ref()
            .ok_or("MIDI CC Learn requires a live audio backend")?;
        let receipt = if name == "learn_midi_cc" {
            if node.bypassed {
                return Err("MIDI CC Learn is unavailable while bypassed".into());
            }
            let cell = args["cell"]
                .as_u64()
                .filter(|cell| (1..=16).contains(cell))
                .ok_or("cell must be an integer from 1 to 16")?;
            backend.learn_midi_cc(id, (cell - 1) as u8)?
        } else {
            backend.cancel_midi_cc_learn(id)?
        };
        Ok(json!({"revision":self.revision,"receipt":receipt}))
    }

    /// Commit before acknowledgment; retain an identity if acknowledgment needs retry.
    pub fn reconcile_midi_cc_learn(&mut self) -> Result<(), String> {
        let Some(backend) = self.live_module_events.clone() else {
            return Ok(());
        };
        let backend_id = Arc::as_ptr(&backend) as *const () as usize;
        let mut ready = backend.midi_cc_learn_ready();
        ready.sort_by(|a, b| (a.frame, a.sequence, &a.id).cmp(&(b.frame, b.sequence, &b.id)));
        for receipt in ready {
            let key = (
                backend_id,
                receipt.id.clone(),
                receipt.instance,
                receipt.epoch,
                receipt.completion.serial,
                receipt.completion.cell,
            );
            if !self.midi_cc_committed.contains(&key) {
                let mut candidate = self.patch.clone();
                let node = candidate
                    .nodes
                    .iter_mut()
                    .find(|node| node.id == receipt.id && node.kind == "midi_cc")
                    .ok_or(
                        "MIDI CC Learn decision targets a missing module; retry reconciliation",
                    )?;
                let Some(SavedModuleState::MidiCc { assignments, .. }) =
                    SavedModuleState::from_node(node)
                else {
                    return Err("invalid MIDI CC Learn state".into());
                };
                if assignments != receipt.completion.before {
                    return Err(
                        "MIDI CC Learn mapping baseline changed; retry reconciliation".into(),
                    );
                }
                SavedModuleState::MidiCcMappings(receipt.completion.after)
                    .write_params(&mut node.params);
                let mut effects = EditEffects::default();
                effects.learn_reconciliation = true;
                effects.before_saved.insert(
                    receipt.id.clone(),
                    SavedModuleState::MidiCcMappings(receipt.completion.before),
                );
                effects.after_saved.insert(
                    receipt.id.clone(),
                    SavedModuleState::MidiCcMappings(receipt.completion.after),
                );
                // The audio decision already applied these mappings on its event sample.
                self.commit(candidate, None, None, &effects)?;
                self.midi_cc_committed.insert(key.clone());
            }
            backend.acknowledge_midi_cc_learn(&receipt)?;
            self.midi_cc_committed.remove(&key);
        }
        Ok(())
    }
}

/// One worker per shared Session, including hundreds of simultaneously armed modules.
pub(super) fn start_worker(shared: &SharedSession, session: &mut Session) {
    if session.midi_cc_worker
        || !session
            .live_module_events
            .as_ref()
            .is_some_and(|backend| backend.midi_cc_learning_active())
    {
        return;
    }
    session.midi_cc_worker = true;
    let weak = Arc::downgrade(shared);
    let spawn = std::thread::Builder::new()
        .name("openpatch-midi-cc-learn".into())
        .spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_millis(10));
            let Some(shared) = weak.upgrade() else {
                break;
            };
            let Ok(mut session) = shared.lock() else {
                break;
            };
            // Failed commits keep the backend's durable Ready slot for a later retry.
            let _ = session.reconcile_midi_cc_learn();
            if !session
                .live_module_events
                .as_ref()
                .is_some_and(|backend| backend.midi_cc_learning_active())
            {
                session.midi_cc_worker = false;
                break;
            }
        });
    if spawn.is_err() {
        session.midi_cc_worker = false;
    }
}
