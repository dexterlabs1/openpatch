//! Bounded control-thread admission and callback-side, acknowledged delivery.
//! Creation, reservations and snapshots are control-thread operations. The
//! reader's scheduling/acknowledgment operations use fixed storage and atomics.
use super::{execution::HostedExecutionGrant, Fence, MAX_EVENTS, QUANTUM};
use crate::project_document::hosted::{HostedCapability, HostedModule, MAX_PARAMETERS};
use crate::project_document::ProjectDocument;
use rtrb::{Consumer, Producer, RingBuffer};
use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering},
    Arc, Mutex, MutexGuard,
};

pub const COMMAND_CAPACITY: usize = 512;
const PENDING_CAPACITY: usize = 4 * MAX_EVENTS;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ParameterAssignment {
    pub parameter: u16,
    pub value: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[repr(u32)]
pub enum ControlFault {
    Closed = 1,
    FrameRegression = 2,
    AcknowledgmentRegression = 3,
    PendingCapacity = 4,
    Sequence = 5,
    Execution = 6,
}

impl ControlFault {
    fn from_code(code: u32) -> Option<Self> {
        match code {
            1 => Some(Self::Closed),
            2 => Some(Self::FrameRegression),
            3 => Some(Self::AcknowledgmentRegression),
            4 => Some(Self::PendingCapacity),
            5 => Some(Self::Sequence),
            6 => Some(Self::Execution),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct ParameterValues {
    /// Parameter IDs in the capability descriptor; unused entries stay zero.
    pub values: [f32; MAX_PARAMETERS],
}
impl ParameterValues {
    pub fn from_module(module: &HostedModule) -> Result<Self, String> {
        module.validate_supported()?;
        let mut values = [0.; MAX_PARAMETERS];
        for (id, parameter) in module.capability()?.parameters().iter().enumerate() {
            values[id] = module.parameter(parameter.key)?;
        }
        Ok(Self { values })
    }
    fn assign(&mut self, parameter: u16, value: f32) {
        self.values[parameter as usize] = value;
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct QueuedReceipt {
    pub first_sequence: u64,
    pub last_sequence: u64,
    pub authored_revision: u64,
    pub commands: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct AppliedParameters {
    pub values: ParameterValues,
    pub sequence: u64,
    pub authored_revision: u64,
    /// Input frame of the last acknowledged command (zero for initial state).
    pub input_frame: u64,
    pub observed_frame: u64,
    pub received_blocks: u64,
}

/// A control-issued admission cut. Only the issuing live control can validate it.
/// Serialization is diagnostic; captures cannot be reconstructed by deserialization.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct HostedCaptureSeal {
    incarnation: u64,
    generation: u64,
    plan_epoch: u64,
    request_id: u64,
    authored_revision: u64,
    accepted_sequence: u64,
}

impl HostedCaptureSeal {
    pub fn fence(&self) -> Fence {
        Fence {
            incarnation: self.incarnation,
            generation: self.generation,
            revision: self.plan_epoch,
        }
    }
    pub fn request_id(&self) -> u64 {
        self.request_id
    }
    pub fn authored_revision(&self) -> u64 {
        self.authored_revision
    }
    pub fn accepted_sequence(&self) -> u64 {
        self.accepted_sequence
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct HostedControlSnapshot {
    pub capability: HostedCapability,
    pub module_id: String,
    pub incarnation: u64,
    pub generation: u64,
    pub plan_epoch: u64,
    pub requested: ParameterValues,
    pub queued: Option<QueuedReceipt>,
    pub latest_accepted_sequence: u64,
    pub outstanding_commands: usize,
    pub reservation_outstanding: bool,
    pub capture: Option<HostedCaptureSeal>,
    pub applied: Option<AppliedParameters>,
    pub available: bool,
    pub failure: Option<ControlFault>,
    pub closed: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScheduledParameter {
    pub parameter: u16,
    pub value: f32,
    pub wire_order: u32,
    pub sequence: u64,
    pub authored_revision: u64,
    pub input_frame: u64,
}

#[derive(Clone, Copy, Default)]
struct Command {
    assignment: ParameterAssignment,
    sequence: u64,
    authored_revision: u64,
}

struct ControlState {
    producer: Producer<Command>,
    reserved: bool,
    requested: ParameterValues,
    queued: Option<QueuedReceipt>,
    last_sequence: u64,
    last_authored_revision: u64,
    last_capture_id: u64,
    capture: Option<HostedCaptureSeal>,
}

/// Single callback writer, control-thread readers. All payload members are
/// atomic, so a reader retry never races non-atomic memory. SeqCst also orders
/// the opening/closing version with every payload access without unsafe code.
struct EffectiveState {
    version: AtomicU64,
    values: [AtomicU32; MAX_PARAMETERS],
    sequence: AtomicU64,
    revision: AtomicU64,
    input_frame: AtomicU64,
    observed_frame: AtomicU64,
    received_blocks: AtomicU64,
}

impl EffectiveState {
    fn new(values: ParameterValues, revision: u64) -> Self {
        Self {
            version: AtomicU64::new(0),
            values: std::array::from_fn(|i| AtomicU32::new(values.values[i].to_bits())),
            sequence: AtomicU64::new(0),
            revision: AtomicU64::new(revision),
            input_frame: AtomicU64::new(0),
            observed_frame: AtomicU64::new(0),
            received_blocks: AtomicU64::new(0),
        }
    }

    fn publish(&self, state: AppliedParameters) {
        self.version.fetch_add(1, Ordering::SeqCst);
        for (target, value) in self.values.iter().zip(state.values.values) {
            target.store(value.to_bits(), Ordering::SeqCst);
        }
        self.sequence.store(state.sequence, Ordering::SeqCst);
        self.revision
            .store(state.authored_revision, Ordering::SeqCst);
        self.input_frame.store(state.input_frame, Ordering::SeqCst);
        self.observed_frame
            .store(state.observed_frame, Ordering::SeqCst);
        self.received_blocks
            .store(state.received_blocks, Ordering::SeqCst);
        self.version.fetch_add(1, Ordering::SeqCst);
    }

    fn snapshot(&self) -> Option<AppliedParameters> {
        loop {
            let before = self.version.load(Ordering::SeqCst);
            if before & 1 != 0 {
                std::hint::spin_loop();
                continue;
            }
            let state = AppliedParameters {
                values: ParameterValues {
                    values: std::array::from_fn(|i| {
                        f32::from_bits(self.values[i].load(Ordering::SeqCst))
                    }),
                },
                sequence: self.sequence.load(Ordering::SeqCst),
                authored_revision: self.revision.load(Ordering::SeqCst),
                input_frame: self.input_frame.load(Ordering::SeqCst),
                observed_frame: self.observed_frame.load(Ordering::SeqCst),
                received_blocks: self.received_blocks.load(Ordering::SeqCst),
            };
            if self.version.load(Ordering::SeqCst) == before {
                return (state.received_blocks > 0).then_some(state);
            }
        }
    }
}

pub struct HostedControl {
    grant: HostedExecutionGrant,
    module_id: String,
    capability: HostedCapability,
    fence: Fence,
    state: Mutex<ControlState>,
    outstanding: AtomicUsize,
    latest_accepted: AtomicU64,
    capture_id: AtomicU64,
    capture_revision: AtomicU64,
    capture_accepted: AtomicU64,
    effective: EffectiveState,
    failure: AtomicU32,
    closed: AtomicBool,
}

impl HostedControl {
    pub fn new(
        grant: HostedExecutionGrant,
        document: &ProjectDocument,
        fence: Fence,
        initial_authored_revision: u64,
    ) -> Result<(Arc<Self>, HostedControlReader), String> {
        if fence.incarnation == 0 || fence.generation == 0 || fence.revision == 0 {
            return Err("hosted control requires a nonzero execution fence".into());
        }
        let plan = grant.prepare(document, fence.generation)?;
        let values = ParameterValues::from_module(plan.module())?;
        let capability = plan.module().capability()?;
        let (producer, consumer) = RingBuffer::new(COMMAND_CAPACITY);
        let shared = Arc::new(Self {
            grant,
            module_id: plan.hosted_id().into(),
            capability,
            fence,
            state: Mutex::new(ControlState {
                producer,
                reserved: false,
                requested: values,
                queued: None,
                last_sequence: 0,
                last_authored_revision: initial_authored_revision,
                last_capture_id: 0,
                capture: None,
            }),
            outstanding: AtomicUsize::new(0),
            latest_accepted: AtomicU64::new(0),
            capture_id: AtomicU64::new(0),
            capture_revision: AtomicU64::new(0),
            capture_accepted: AtomicU64::new(0),
            effective: EffectiveState::new(values, initial_authored_revision),
            failure: AtomicU32::new(0),
            closed: AtomicBool::new(false),
        });
        let reader = HostedControlReader {
            shared: Arc::clone(&shared),
            consumer,
            pending: [ScheduledParameter::default(); PENDING_CAPACITY],
            pending_head: 0,
            pending_len: 0,
            last_frame: None,
            quantum: None,
            quantum_commands: 0,
            last_sequence: 0,
            last_ack_frame: None,
            applied: AppliedParameters {
                values,
                sequence: 0,
                authored_revision: initial_authored_revision,
                input_frame: 0,
                observed_frame: 0,
                received_blocks: 0,
            },
        };
        Ok((shared, reader))
    }

    fn state(&self) -> MutexGuard<'_, ControlState> {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    pub fn fence(&self) -> Fence {
        self.fence
    }

    pub fn capability(&self) -> HostedCapability {
        self.capability
    }
    pub fn module_id(&self) -> &str {
        &self.module_id
    }

    pub fn accepts_document(
        &self,
        document: &ProjectDocument,
        generation: u64,
    ) -> Result<bool, String> {
        if self.unavailable().is_some() {
            return Ok(false);
        }
        self.grant.matches_document(document, generation)
    }

    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
    }

    /// Lock-free execution lease check for the prepared audio plan.
    pub fn is_open(&self) -> bool {
        self.unavailable().is_none()
    }

    fn unavailable(&self) -> Option<ControlFault> {
        ControlFault::from_code(self.failure.load(Ordering::Acquire)).or_else(|| {
            self.closed
                .load(Ordering::Acquire)
                .then_some(ControlFault::Closed)
        })
    }

    /// Seal admission without revoking the audio execution lease. A reservation
    /// must either publish or be dropped before this cut can be established.
    pub fn begin_capture(&self, authored_revision: u64) -> Result<HostedCaptureSeal, String> {
        let mut state = self.state();
        if self.unavailable().is_some() {
            return Err("hosted control is unavailable".into());
        }
        if state.reserved {
            return Err("a hosted control reservation is outstanding".into());
        }
        if state.capture.is_some() {
            return Err("a hosted capture is already active".into());
        }
        if authored_revision < state.last_authored_revision {
            return Err("hosted capture authored revision precedes accepted state".into());
        }
        let request_id = state
            .last_capture_id
            .checked_add(1)
            .ok_or("hosted capture request sequence exhausted")?;
        let seal = HostedCaptureSeal {
            incarnation: self.fence.incarnation,
            generation: self.fence.generation,
            plan_epoch: self.fence.revision,
            request_id,
            authored_revision,
            accepted_sequence: state.last_sequence,
        };
        state.last_capture_id = request_id;
        state.capture = Some(seal);
        self.capture_revision
            .store(authored_revision, Ordering::Release);
        self.capture_accepted
            .store(seal.accepted_sequence, Ordering::Release);
        self.capture_id.store(request_id, Ordering::Release);
        Ok(seal)
    }

    pub fn cancel_capture(&self, seal: HostedCaptureSeal) -> Result<(), String> {
        let mut state = self.state();
        self.validate_capture_state(&state, &seal)?;
        self.capture_id.store(0, Ordering::Release);
        state.capture = None;
        Ok(())
    }

    fn validate_capture_state(
        &self,
        state: &ControlState,
        seal: &HostedCaptureSeal,
    ) -> Result<(), String> {
        if self.unavailable().is_some()
            || state.capture != Some(*seal)
            || seal.fence() != self.fence
        {
            return Err("hosted capture seal is stale or control is unavailable".into());
        }
        Ok(())
    }

    pub fn reserve(
        self: &Arc<Self>,
        assignments: &[ParameterAssignment],
    ) -> Result<HostedReservation, String> {
        if assignments.is_empty() || assignments.len() > MAX_EVENTS {
            return Err("hosted assignment batch requires 1..64 commands".into());
        }
        if assignments
            .iter()
            .any(|a| !self.capability.accepts_parameter(a.parameter, a.value))
        {
            return Err("unsupported hosted parameter or value".into());
        }
        let mut state = self.state();
        if self.unavailable().is_some() {
            return Err("hosted control is unavailable".into());
        }
        if state.capture.is_some() {
            return Err("hosted control admission is sealed for capture".into());
        }
        if state.reserved {
            return Err("a hosted control reservation is already outstanding".into());
        }
        if state
            .last_sequence
            .checked_add(assignments.len() as u64)
            .is_none()
        {
            return Err("hosted command sequence exhausted".into());
        }
        if self.outstanding.load(Ordering::Acquire) + assignments.len() > COMMAND_CAPACITY
            || state.producer.slots() < assignments.len()
        {
            return Err("hosted command queue is full".into());
        }
        state.reserved = true;
        let mut values = [ParameterAssignment::default(); MAX_EVENTS];
        values[..assignments.len()].copy_from_slice(assignments);
        Ok(HostedReservation {
            shared: Arc::clone(self),
            assignments: values,
            len: assignments.len(),
            published: false,
        })
    }

    pub fn snapshot(&self) -> HostedControlSnapshot {
        let state = self.state();
        let applied = self.effective.snapshot();
        let failure = ControlFault::from_code(self.failure.load(Ordering::Acquire));
        let closed = self.closed.load(Ordering::Acquire);
        HostedControlSnapshot {
            capability: self.capability,
            module_id: self.module_id.clone(),
            incarnation: self.fence.incarnation,
            generation: self.fence.generation,
            plan_epoch: self.fence.revision,
            requested: state.requested,
            queued: state.queued,
            latest_accepted_sequence: state.last_sequence,
            outstanding_commands: self.outstanding.load(Ordering::Acquire),
            reservation_outstanding: state.reserved,
            capture: state.capture,
            applied,
            available: applied.is_some() && failure.is_none() && !closed,
            failure,
            closed,
        }
    }
}

/// Exactly one reservation may exist. Dropping it returns admission capacity
/// without allocating a command sequence or changing requested values.
pub struct HostedReservation {
    shared: Arc<HostedControl>,
    assignments: [ParameterAssignment; MAX_EVENTS],
    len: usize,
    published: bool,
}

impl HostedReservation {
    /// Infallible after successful admission, including retirement races. Such
    /// commands remain accepted/queued but are unavailable and never false-acked.
    pub fn publish(mut self, authored_revision: u64) -> QueuedReceipt {
        let mut state = self.shared.state();
        let receipt = QueuedReceipt {
            first_sequence: state.last_sequence + 1,
            last_sequence: state.last_sequence + self.len as u64,
            authored_revision,
            commands: self.len,
        };
        self.shared
            .outstanding
            .fetch_add(self.len, Ordering::AcqRel);
        // Publish acceptance before exposing any command to the callback.
        self.shared
            .latest_accepted
            .store(receipt.last_sequence, Ordering::Release);
        for (index, assignment) in self.assignments[..self.len].iter().copied().enumerate() {
            state
                .requested
                .assign(assignment.parameter, assignment.value);
            let command = Command {
                assignment,
                sequence: receipt.first_sequence + index as u64,
                authored_revision,
            };
            // Reservation excluded every competing producer and reserved at
            // least this many slots. Only the consumer can free slots meanwhile.
            assert!(
                state.producer.push(command).is_ok(),
                "reserved ring capacity"
            );
        }
        state.last_sequence = receipt.last_sequence;
        state.last_authored_revision = state.last_authored_revision.max(authored_revision);
        state.queued = Some(receipt);
        state.reserved = false;
        self.published = true;
        receipt
    }
}

impl Drop for HostedReservation {
    fn drop(&mut self) {
        if !self.published {
            self.shared.state().reserved = false;
        }
    }
}

/// Unique callback reader. Keep ownership in the prepared/retired plan and
/// destroy it on a control thread, like the hosted endpoint itself.
pub struct HostedControlReader {
    shared: Arc<HostedControl>,
    consumer: Consumer<Command>,
    pending: [ScheduledParameter; PENDING_CAPACITY],
    pending_head: usize,
    pending_len: usize,
    last_frame: Option<u64>,
    quantum: Option<u64>,
    quantum_commands: usize,
    last_sequence: u64,
    last_ack_frame: Option<u64>,
    applied: AppliedParameters,
}

impl HostedControlReader {
    pub fn capability(&self) -> HostedCapability {
        self.shared.capability
    }
    pub fn fence(&self) -> Fence {
        self.shared.fence
    }

    /// Callback-side seal check. Every payload field is atomic and a second ID
    /// read rejects cancellation/resealing while checking the cut.
    pub fn capture_matches(&self, seal: &HostedCaptureSeal) -> bool {
        self.shared.unavailable().is_none()
            && seal.fence() == self.shared.fence
            && self.shared.capture_id.load(Ordering::Acquire) == seal.request_id
            && self.shared.capture_revision.load(Ordering::Acquire) == seal.authored_revision
            && self.shared.capture_accepted.load(Ordering::Acquire) == seal.accepted_sequence
            && self.shared.capture_id.load(Ordering::Acquire) == seal.request_id
    }

    pub fn scheduled_sequence(&self) -> u64 {
        self.last_sequence
    }

    pub(crate) fn validate_capture(&self, seal: &HostedCaptureSeal) -> Result<(), String> {
        let state = self.shared.state();
        self.shared.validate_capture_state(&state, seal)
    }

    /// Only the retained reader may certify its complete, validated ack history.
    /// Closing under the admission mutex makes completion atomic against cancel.
    pub(crate) fn finish_capture(
        &self,
        seal: &HostedCaptureSeal,
        received_blocks: u64,
    ) -> Result<u64, String> {
        let state = self.shared.state();
        self.shared.validate_capture_state(&state, seal)?;
        if state.reserved
            || state.last_sequence != seal.accepted_sequence
            || self.last_sequence != seal.accepted_sequence
            || self.applied.sequence != seal.accepted_sequence
            || self.applied.received_blocks != received_blocks
            || self.pending_len != 0
            || self.shared.outstanding.load(Ordering::Acquire) != 0
        {
            return Err("hosted capture commands are not fully acknowledged".into());
        }
        self.shared.close();
        Ok(self.applied.sequence)
    }

    pub fn next_for_sample(
        &mut self,
        frame: u64,
    ) -> Result<Option<ScheduledParameter>, ControlFault> {
        if let Some(fault) = self.shared.unavailable() {
            return Err(fault);
        }
        if self.last_frame.is_some_and(|previous| frame < previous)
            || frame / (QUANTUM as u64) < self.applied.received_blocks
        {
            return Err(self.fail(ControlFault::FrameRegression));
        }
        self.last_frame = Some(frame);
        let quantum = frame / QUANTUM as u64;
        if self.quantum != Some(quantum) {
            self.quantum = Some(quantum);
            self.quantum_commands = 0;
        }
        if self.quantum_commands == MAX_EVENTS {
            return Ok(None);
        }
        if self.pending_len == PENDING_CAPACITY {
            // Empty polls remain harmless; a command would exceed the four
            // transport-slot acknowledgment window and must suspend execution.
            if self.consumer.slots() == 0 {
                return Ok(None);
            }
            return Err(self.fail(ControlFault::PendingCapacity));
        }
        let Ok(command) = self.consumer.pop() else {
            return Ok(None);
        };
        if command.sequence != self.last_sequence + 1
            || command.sequence > self.shared.latest_accepted.load(Ordering::Acquire)
        {
            return Err(self.fail(ControlFault::Sequence));
        }
        let scheduled = ScheduledParameter {
            parameter: command.assignment.parameter,
            value: command.assignment.value,
            wire_order: self.quantum_commands as u32,
            sequence: command.sequence,
            authored_revision: command.authored_revision,
            input_frame: frame,
        };
        let tail = (self.pending_head + self.pending_len) % PENDING_CAPACITY;
        self.pending[tail] = scheduled;
        self.pending_len += 1;
        self.quantum_commands += 1;
        self.last_sequence = command.sequence;
        Ok(Some(scheduled))
    }

    /// Caller supplies only the parent endpoint's validated received-block count.
    /// Scheduling or submission alone cannot establish effective plugin state.
    pub fn acknowledge(
        &mut self,
        received_blocks: u64,
        observed_frame: u64,
    ) -> Result<(), ControlFault> {
        if let Some(fault) = self.shared.unavailable() {
            return Err(fault);
        }
        if received_blocks < self.applied.received_blocks
            || self
                .last_ack_frame
                .is_some_and(|previous| observed_frame < previous)
        {
            return Err(self.fail(ControlFault::AcknowledgmentRegression));
        }
        self.last_ack_frame = Some(observed_frame);
        let received_advanced = received_blocks > self.applied.received_blocks;
        let mut acknowledged = 0;
        while self.pending_len > 0 {
            let event = self.pending[self.pending_head];
            if event.input_frame / QUANTUM as u64 >= received_blocks {
                break;
            }
            self.applied.values.assign(event.parameter, event.value);
            self.applied.sequence = event.sequence;
            self.applied.authored_revision = event.authored_revision;
            self.applied.input_frame = event.input_frame;
            self.pending_head = (self.pending_head + 1) % PENDING_CAPACITY;
            self.pending_len -= 1;
            acknowledged += 1;
        }
        if received_advanced || acknowledged > 0 {
            self.applied.received_blocks = received_blocks;
            self.applied.observed_frame = observed_frame;
            self.shared.effective.publish(self.applied);
        }
        if acknowledged > 0 {
            self.shared
                .outstanding
                .fetch_sub(acknowledged, Ordering::AcqRel);
        }
        Ok(())
    }

    pub fn fail(&mut self, fault: ControlFault) -> ControlFault {
        let _ = self.shared.failure.compare_exchange(
            0,
            fault as u32,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        ControlFault::from_code(self.shared.failure.load(Ordering::Acquire)).unwrap_or(fault)
    }

    pub fn close(&mut self) {
        self.shared.close();
    }
}

impl Drop for HostedControlReader {
    fn drop(&mut self) {
        self.shared.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair() -> (Arc<HostedControl>, HostedControlReader) {
        let document = super::super::graph::tests::document();
        HostedControl::new(
            HostedExecutionGrant::new(&document, 7).unwrap(),
            &document,
            Fence {
                incarnation: 1,
                generation: 7,
                revision: 9,
            },
            42,
        )
        .unwrap()
    }

    fn assignment(parameter: u16, value: f32) -> ParameterAssignment {
        ParameterAssignment { parameter, value }
    }

    #[test]
    fn cancel_and_rejected_batches_do_not_consume_sequences_or_values() {
        let (control, _reader) = pair();
        let reservation = control.reserve(&[assignment(0, 0.25)]).unwrap();
        assert!(control.reserve(&[assignment(0, 0.5)]).is_err());
        drop(reservation);
        for batch in [
            vec![],
            vec![assignment(2, 0.)],
            vec![assignment(0, f32::NAN)],
            vec![assignment(1, 1.01)],
            vec![assignment(0, 0.); 65],
        ] {
            assert!(control.reserve(&batch).is_err());
        }
        let snapshot = control.snapshot();
        assert_eq!(snapshot.latest_accepted_sequence, 0);
        assert_eq!(snapshot.requested.values[0], 0.);
        assert_eq!(
            control
                .reserve(&[assignment(0, 0.25)])
                .unwrap()
                .publish(10)
                .first_sequence,
            1
        );
    }

    #[test]
    fn saturation_counts_pending_and_frees_only_after_matching_ack() {
        let (control, mut reader) = pair();
        let batch = [assignment(0, 0.25); MAX_EVENTS];
        for revision in 10..18 {
            control.reserve(&batch).unwrap().publish(revision);
        }
        assert!(control.reserve(&[assignment(0, 0.)]).is_err());
        for index in 0..MAX_EVENTS {
            let event = reader.next_for_sample(12).unwrap().unwrap();
            assert_eq!(event.sequence, index as u64 + 1);
            assert_eq!(event.wire_order, index as u32);
            assert_eq!(event.input_frame, 12);
        }
        assert!(reader.next_for_sample(127).unwrap().is_none());
        assert!(control.reserve(&batch).is_err());
        reader.acknowledge(0, 127).unwrap();
        assert!(control.snapshot().applied.is_none());
        assert!(control.reserve(&batch).is_err());
        reader.acknowledge(1, 384).unwrap();
        assert_eq!(control.snapshot().applied.unwrap().sequence, 64);
        assert_eq!(control.snapshot().outstanding_commands, 448);
        assert_eq!(
            control.reserve(&batch).unwrap().publish(18).last_sequence,
            576
        );
        assert_eq!(reader.next_for_sample(128).unwrap().unwrap().sequence, 65);
    }

    #[test]
    fn equal_assignments_keep_fifo_and_effective_pair_waits_for_ack() {
        let (control, mut reader) = pair();
        control
            .reserve(&[
                assignment(0, 0.25),
                assignment(0, 0.25),
                assignment(1, -0.75),
            ])
            .unwrap()
            .publish(22);
        let first = reader.next_for_sample(127).unwrap().unwrap();
        let second = reader.next_for_sample(127).unwrap().unwrap();
        let third = reader.next_for_sample(128).unwrap().unwrap();
        assert_eq!((first.sequence, second.sequence, third.sequence), (1, 2, 3));
        assert_eq!(
            (first.wire_order, second.wire_order, third.wire_order),
            (0, 1, 0)
        );
        assert_eq!(
            control.snapshot().requested,
            ParameterValues {
                values: [0.25, -0.75, 0.]
            }
        );
        reader.acknowledge(1, 384).unwrap();
        let applied = control.snapshot().applied.unwrap();
        assert_eq!(applied.sequence, 2);
        assert_eq!(
            applied.values,
            ParameterValues {
                values: [0.25, 0.5, 0.]
            }
        );
        assert_eq!(applied.authored_revision, 22);
        assert_eq!(applied.input_frame, 127);
        reader.acknowledge(2, 512).unwrap();
        let applied = control.snapshot().applied.unwrap();
        assert_eq!(applied.sequence, 3);
        assert_eq!(applied.values.values[1], -0.75);
        assert_eq!(applied.input_frame, 128);
    }

    #[test]
    fn initial_state_needs_validated_output_and_retirement_never_acks() {
        let (control, mut reader) = pair();
        assert!(!control.snapshot().available);
        reader.acknowledge(1, 384).unwrap();
        assert!(control.snapshot().available);
        assert_eq!(control.snapshot().applied.unwrap().sequence, 0);
        let reservation = control.reserve(&[assignment(0, 1.)]).unwrap();
        control.close();
        assert_eq!(reservation.publish(10).last_sequence, 1);
        assert_eq!(reader.next_for_sample(0), Err(ControlFault::Closed));
        assert_eq!(reader.acknowledge(2, 512), Err(ControlFault::Closed));
        let snapshot = control.snapshot();
        assert!(snapshot.closed);
        assert!(!snapshot.available);
        assert_eq!(snapshot.applied.unwrap().sequence, 0);
        assert_eq!(snapshot.latest_accepted_sequence, 1);
        assert_eq!(snapshot.requested.values[0], 1.);
        assert_eq!(snapshot.queued.unwrap().last_sequence, 1);
        assert_eq!(snapshot.outstanding_commands, 1);
        assert!(control.reserve(&[assignment(0, 0.)]).is_err());
    }

    #[test]
    fn unchanged_ack_poll_keeps_the_last_validated_observation_frame() {
        let (control, mut reader) = pair();
        reader.acknowledge(0, 127).unwrap();
        assert!(control.snapshot().applied.is_none());
        reader.acknowledge(1, 384).unwrap();
        let first = control.snapshot().applied.unwrap();
        reader.acknowledge(1, 400).unwrap();
        reader.acknowledge(1, 511).unwrap();
        assert_eq!(control.snapshot().applied.unwrap(), first);
        reader.acknowledge(2, 512).unwrap();
        let next = control.snapshot().applied.unwrap();
        assert_eq!(next.observed_frame, 512);
        assert_eq!(next.received_blocks, 2);
        assert_eq!(next.values, first.values);
        assert_eq!(next.sequence, first.sequence);
        reader.acknowledge(2, 600).unwrap();
        assert_eq!(control.snapshot().applied.unwrap(), next);
        // Every poll still advances the monotonic-call guard even though the
        // effective observation retains the frame of the latest real output.
        assert_eq!(
            reader.acknowledge(2, 599),
            Err(ControlFault::AcknowledgmentRegression)
        );
        assert_eq!(control.snapshot().applied.unwrap(), next);
    }

    #[test]
    fn reader_drop_faults_and_regressions_are_visible_and_latched() {
        let (control, reader) = pair();
        drop(reader);
        assert!(control.snapshot().closed);
        let (control, mut reader) = pair();
        reader.next_for_sample(128).unwrap();
        assert_eq!(
            reader.next_for_sample(127),
            Err(ControlFault::FrameRegression)
        );
        assert_eq!(
            reader.fail(ControlFault::Execution),
            ControlFault::FrameRegression
        );
        assert_eq!(
            reader.acknowledge(1, 384),
            Err(ControlFault::FrameRegression)
        );
        assert_eq!(
            control.snapshot().failure,
            Some(ControlFault::FrameRegression)
        );
        let (control, mut reader) = pair();
        reader.acknowledge(1, 384).unwrap();
        assert_eq!(
            reader.acknowledge(0, 385),
            Err(ControlFault::AcknowledgmentRegression)
        );
        assert!(!control.snapshot().available);
    }

    #[test]
    fn full_pending_window_faults_without_claiming_unsent_work_applied() {
        let (control, mut reader) = pair();
        let batch = [assignment(0, 0.25); MAX_EVENTS];
        for revision in 10..15 {
            control.reserve(&batch).unwrap().publish(revision);
        }
        for block in 0..4 {
            for _ in 0..MAX_EVENTS {
                reader
                    .next_for_sample(block * QUANTUM as u64)
                    .unwrap()
                    .unwrap();
            }
        }
        assert_eq!(
            reader.next_for_sample(512),
            Err(ControlFault::PendingCapacity)
        );
        assert!(control.snapshot().applied.is_none());
        assert_eq!(control.snapshot().outstanding_commands, 320);
    }

    #[test]
    fn grant_generation_and_full_topology_are_checked() {
        let (control, _reader) = pair();
        let mut document = super::super::graph::tests::document();
        assert!(control.accepts_document(&document, 7).unwrap());
        assert!(!control.accepts_document(&document, 8).unwrap());
        document.graph.cables.clear();
        assert!(control.accepts_document(&document, 7).is_err());
        let original = super::super::graph::tests::document();
        assert!(HostedControl::new(
            HostedExecutionGrant::new(&original, 7).unwrap(),
            &original,
            Fence {
                incarnation: 1,
                generation: 8,
                revision: 1
            },
            42,
        )
        .is_err());
    }

    #[test]
    fn sequence_exhaustion_is_rejected_before_reservation() {
        let (control, _reader) = pair();
        control.state().last_sequence = u64::MAX - 1;
        assert!(control.reserve(&[assignment(0, 0.); 2]).is_err());
        assert!(!control.snapshot().reservation_outstanding);
        assert_eq!(
            control
                .reserve(&[assignment(0, 0.)])
                .unwrap()
                .publish(10)
                .last_sequence,
            u64::MAX
        );
        assert!(control.reserve(&[assignment(0, 0.)]).is_err());
    }

    #[test]
    fn initial_authored_revision_is_distinct_from_the_plan_epoch() {
        let (control, mut reader) = pair();
        assert_eq!(control.fence().revision, 9);
        assert_eq!(control.snapshot().plan_epoch, 9);
        reader.acknowledge(1, 384).unwrap();
        let applied = control.snapshot().applied.unwrap();
        assert_eq!(applied.authored_revision, 42);
        assert_eq!(applied.sequence, 0);
    }

    #[test]
    fn poisoned_control_mutex_does_not_break_an_admitted_publication() {
        let (control, mut reader) = pair();
        let reservation = control.reserve(&[assignment(1, -0.25)]).unwrap();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _state = control.state();
            panic!("control-thread interruption");
        }));
        assert_eq!(reservation.publish(10).last_sequence, 1);
        assert_eq!(reader.next_for_sample(0).unwrap().unwrap().value, -0.25);
        reader.acknowledge(1, 384).unwrap();
        assert_eq!(control.snapshot().applied.unwrap().values.values[1], -0.25);
    }

    #[test]
    fn scheduling_cannot_reuse_an_already_acknowledged_input_block() {
        let (control, mut reader) = pair();
        reader.acknowledge(1, 384).unwrap();
        control.reserve(&[assignment(0, 0.25)]).unwrap().publish(10);
        assert_eq!(
            reader.next_for_sample(127),
            Err(ControlFault::FrameRegression)
        );
        assert_eq!(control.snapshot().applied.unwrap().sequence, 0);
    }
    #[test]
    fn capture_seals_admission_without_revoking_execution_and_exact_cancel_reopens_it() {
        let (control, reader) = pair();
        assert!(control.begin_capture(41).is_err());
        let reservation = control.reserve(&[assignment(0, 0.25)]).unwrap();
        assert!(control.begin_capture(42).is_err());
        assert!(control.snapshot().capture.is_none());
        reservation.publish(43);
        assert!(control.begin_capture(42).is_err());
        let seal = control.begin_capture(43).unwrap();
        assert_eq!(seal.accepted_sequence(), 1);
        assert!(control.is_open());
        assert!(control.reserve(&[assignment(1, 0.5)]).is_err());
        assert_eq!(control.snapshot().capture, Some(seal));
        // A callback's two queries must also work while the control mutex is held.
        {
            let _held = control.state();
            assert!(reader.capture_matches(&seal));
            assert_eq!(reader.scheduled_sequence(), 0);
        }
        let mut wrong = seal;
        wrong.plan_epoch += 1;
        assert!(control.cancel_capture(wrong).is_err());
        assert!(reader.capture_matches(&seal));
        control.cancel_capture(seal).unwrap();
        assert!(!reader.capture_matches(&seal));
        let newer = control.begin_capture(44).unwrap();
        assert!(newer.request_id() > seal.request_id());
        assert!(control.cancel_capture(seal).is_err());
        assert!(reader.capture_matches(&newer));
        control.cancel_capture(newer).unwrap();
        assert!(control.reserve(&[assignment(0, 0.5)]).is_ok());
    }

    #[test]
    fn capture_and_reservation_race_has_exactly_one_admission_winner() {
        use std::sync::Barrier;
        for _ in 0..16 {
            let (control, _reader) = pair();
            let barrier = Arc::new(Barrier::new(2));
            let producer = Arc::clone(&control);
            let start = Arc::clone(&barrier);
            let worker = std::thread::spawn(move || {
                start.wait();
                producer.reserve(&[assignment(0, 0.25)])
            });
            barrier.wait();
            let capture = control.begin_capture(42);
            let reservation = worker.join().unwrap();
            assert_ne!(capture.is_ok(), reservation.is_ok());
            if let Ok(reservation) = reservation {
                reservation.publish(43);
                assert_eq!(control.begin_capture(43).unwrap().accepted_sequence(), 1);
            } else {
                assert_eq!(capture.unwrap().accepted_sequence(), 0);
            }
        }
    }

    #[test]
    fn capture_request_exhaustion_and_closed_control_never_issue_a_seal() {
        let (control, _reader) = pair();
        control.state().last_capture_id = u64::MAX;
        assert!(control.begin_capture(42).is_err());
        assert!(control.snapshot().capture.is_none());
        assert!(control.is_open());
        let (control, replacement) = pair();
        drop(replacement);
        assert!(control.begin_capture(42).is_err());
    }
}
