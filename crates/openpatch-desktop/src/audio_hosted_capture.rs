//! Two-phase full-graph quiescence. Queues move ownership; audio never waits,
//! allocates capture storage, serializes state, or destroys a returned graph.
use super::*;
use openpatch::binary_host::control::HostedCaptureSeal;
use openpatch::binary_host::runner::QuiescedHostedRunner;
use openpatch::control::{HostedProjectCaptureReceipt, HostedProjectCaptureTicket};
use std::time::{Duration, Instant};

const CAPTURE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug)]
enum PacketState {
    Requested,
    Frozen { device_frame: u64 },
    Refused,
}
struct CapturePacket {
    seal: HostedCaptureSeal,
    graph: PreparedGraph,
    state: PacketState,
}
struct DrainingCapture {
    packet: Box<CapturePacket>,
    runner: QuiescedHostedRunner,
}
struct PendingCapture {
    ticket: HostedProjectCaptureTicket,
    seal: HostedCaptureSeal,
    started: Instant,
    native_intent: Vec<u64>,
    cancel_error: Option<String>,
    draining: Option<DrainingCapture>,
}
pub(super) struct CaptureControl {
    requests: Producer<Box<CapturePacket>>,
    returns: Consumer<Box<CapturePacket>>,
    pending: Option<PendingCapture>,
    ready: Option<(u64, Result<HostedProjectCaptureReceipt, String>)>,
    resume_error: Option<String>,
    dispatched: Option<(
        u64,
        std::sync::mpsc::SyncSender<Result<HostedProjectCaptureReceipt, String>>,
    )>,
    pub(super) recovery: Option<PreparedGraph>,
}
pub(super) struct CaptureCallback {
    requests: Consumer<Box<CapturePacket>>,
    returns: Producer<Box<CapturePacket>>,
    pending: Option<Box<CapturePacket>>,
    held_return: Option<Box<CapturePacket>>,
    control: Option<CaptureControl>,
}
impl CaptureControl {
    pub(super) fn busy(&self) -> bool {
        self.pending.is_some() || self.ready.is_some()
    }
}
impl CaptureCallback {
    pub(super) fn new() -> Self {
        let (send, requests) = RingBuffer::new(1);
        let (returns, receive) = RingBuffer::new(1);
        Self {
            requests,
            returns,
            pending: None,
            held_return: None,
            control: Some(CaptureControl {
                requests: send,
                returns: receive,
                pending: None,
                ready: None,
                resume_error: None,
                dispatched: None,
                recovery: None,
            }),
        }
    }
    pub(super) fn take_control(&mut self) -> CaptureControl {
        self.control.take().expect("unique capture controller")
    }
    fn return_packet(&mut self, packet: Box<CapturePacket>) {
        debug_assert!(self.held_return.is_none());
        if let Err(PushError::Full(packet)) = self.returns.push(packet) {
            self.held_return = Some(packet);
        }
    }
}
impl Callback {
    pub(super) fn poll_hosted_capture(&mut self) -> bool {
        if let Some(packet) = self.capture.held_return.take() {
            self.capture.return_packet(packet);
        }
        if self.capture.held_return.is_some() {
            return true;
        }
        if self.capture.pending.is_none() {
            self.capture.pending = self.capture.requests.pop().ok();
        }
        let valid = self.capture.pending.as_ref().is_none_or(|packet| {
            self.graphs.active.current_revision() == packet.seal.authored_revision()
                && self.graphs.active.hosted.as_ref().is_some_and(|runner| {
                    runner.fence() == packet.seal.fence()
                        && runner.capture_matches(&packet.seal)
                        && runner.fault().is_none()
                })
        });
        if !valid {
            let mut packet = self.capture.pending.take().unwrap();
            packet.state = PacketState::Refused;
            self.capture.return_packet(packet);
        }
        self.capture.pending.is_some()
    }
    pub(super) fn capture_span(&self, requested: usize) -> usize {
        if self.capture.pending.is_none() {
            return requested;
        }
        self.graphs
            .active
            .hosted
            .as_ref()
            .map_or(requested, |runner| {
                let quantum = openpatch::binary_host::QUANTUM;
                requested.min(quantum - runner.frame() as usize % quantum)
            })
    }
    pub(super) fn try_hosted_capture(&mut self) -> bool {
        let ready = self.graphs.previous.is_none()
            && self.capture.pending.as_ref().is_some_and(|packet| {
                self.graphs.active.current_revision() == packet.seal.authored_revision()
                    && self
                        .graphs
                        .active
                        .hosted
                        .as_ref()
                        .is_some_and(|runner| runner.capture_ready(&packet.seal))
            });
        if !ready {
            return false;
        }
        let mut packet = self.capture.pending.take().unwrap();
        // Only pointers/fixed fields move. The old native Engine is now frozen
        // and cannot be advanced by callback silence or control-side draining.
        std::mem::swap(&mut packet.graph, &mut self.graphs.active);
        packet.state = PacketState::Frozen {
            device_frame: self.frame_clock,
        };
        self.capture.return_packet(packet);
        true
    }
}

impl Audio {
    pub(super) fn capture_in_progress(&self) -> bool {
        self.capture.pending.is_some()
    }
    pub(crate) fn begin_hosted_capture(
        &mut self,
        ticket: HostedProjectCaptureTicket,
    ) -> Result<u64, String> {
        if self.capture.pending.is_some() || self.capture.ready.is_some() {
            return Err("another hosted capture must be completed or collected first".into());
        }
        let accepted = self
            .accepted_document
            .as_ref()
            .ok_or("audio has no accepted full document")?;
        if ticket.revision() != self.submitted_revision
            || ticket.document_generation() != accepted.document_generation
            || serde_json::to_value(&ticket.document().graph).map_err(|e| e.to_string())?
                != serde_json::to_value(&accepted.document.graph).map_err(|e| e.to_string())?
            || self.pending.is_some()
            || self.incoming.slots() != QUEUE_SIZE
            || !self.performance.execution_available()
        {
            return Err(
                "hosted capture requires the current active, fully submitted document".into(),
            );
        }
        let control = self
            .hosted_control
            .as_ref()
            .ok_or("audio has no live hosted control")?;
        // Allocate/compile the silence replacement before sealing admissions.
        let patch: Patch = serde_json::from_value(serde_json::json!({"sample_rate":48000,
            "nodes":[{"id":"capture_silence","kind":"constant","params":{"value":0.}}],
            "output":"capture_silence"}))
        .map_err(|e| e.to_string())?;
        let graph = PreparedGraph::from(patch.compile()?);
        let seal = control.begin_capture(ticket.revision())?;
        let packet = Box::new(CapturePacket {
            seal,
            graph,
            state: PacketState::Requested,
        });
        if let Err(PushError::Full(packet)) = self.capture.requests.push(packet) {
            control.cancel_capture(seal)?;
            drop(packet);
            return Err("hosted capture request queue is full".into());
        }
        self.capture.resume_error = None;
        self.capture.pending = Some(PendingCapture {
            ticket,
            seal,
            started: Instant::now(),
            native_intent: self.performance.native_intent_stamp(),
            draining: None,
            cancel_error: None,
        });
        self.performance.set_hosted_capture_pending(true);
        Ok(seal.request_id())
    }

    pub(crate) fn take_hosted_capture_result(
        &mut self,
        request_id: u64,
    ) -> Option<Result<HostedProjectCaptureReceipt, String>> {
        if self
            .capture
            .ready
            .as_ref()
            .is_some_and(|(id, _)| *id == request_id)
        {
            self.capture.ready.take().map(|(_, result)| result)
        } else {
            None
        }
    }

    pub(super) fn service_hosted_capture(&mut self) {
        if let Some((ticket, send)) = self.performance.take_hosted_capture_dispatch() {
            match self.begin_hosted_capture(ticket) {
                Ok(id) => self.capture.dispatched = Some((id, send)),
                Err(error) => {
                    let _ = send.try_send(Err(error));
                }
            }
        }
        if let Some((id, _)) = self.capture.dispatched.as_ref() {
            if let Some(result) = self.take_hosted_capture_result(*id) {
                let (_, send) = self.capture.dispatched.take().unwrap();
                let _ = send.try_send(result);
            }
        }

        let Some(mut pending) = self.capture.pending.take() else {
            while let Ok(packet) = self.capture.returns.pop() {
                self.dispose_graph(packet.graph);
            }
            return;
        };
        if pending.draining.is_none() {
            if let Ok(mut packet) = self.capture.returns.pop() {
                if packet.seal != pending.seal {
                    self.dispose_graph(packet.graph);
                    self.capture.pending = Some(pending);
                    return;
                }
                if pending.cancel_error.is_some() {
                    if matches!(packet.state, PacketState::Frozen { .. })
                        && self.capture_still_current(&pending)
                        && self
                            .hosted_control
                            .as_ref()
                            .is_some_and(|control| control.is_open())
                        && self.pending.is_none()
                    {
                        // Cancellation raced the swap. Return the exact runner
                        // and frozen native state to audio, without a restart.
                        packet.graph.resumed_from_capture = true;
                        self.pending = Some(packet.graph);
                    } else if matches!(packet.state, PacketState::Frozen { .. }) {
                        self.retain_capture_recovery(packet.graph);
                    } else {
                        self.dispose_graph(packet.graph);
                    }
                    self.performance.set_hosted_capture_pending(false);
                    return;
                }
                if !matches!(packet.state, PacketState::Frozen { .. }) {
                    self.fail_hosted_capture(
                        pending,
                        "capture target was replaced, canceled or suspended".into(),
                    );
                    self.dispose_graph(packet.graph);
                    return;
                }
                let runner = packet.graph.hosted.take().expect("frozen hosted graph");
                match runner.quiesce(pending.seal) {
                    Ok(runner) => pending.draining = Some(DrainingCapture { packet, runner }),
                    Err((runner, error)) => {
                        packet.graph.hosted = Some(runner);
                        self.retain_capture_recovery(packet.graph);
                        self.fail_hosted_capture(pending, error);
                        return;
                    }
                }
            }
        }
        if pending.cancel_error.is_none() && pending.started.elapsed() > CAPTURE_TIMEOUT {
            let error = "hosted capture timed out without a coherent receipt".to_string();
            if pending.draining.is_none() {
                // Report failure, but retain the terminal request until audio
                // returns ownership. A callback can have passed its cut check
                // concurrently with cancellation; dropping that graph loses DSP.
                if let Some(control) = &self.hosted_control {
                    let _ = control.cancel_capture(pending.seal);
                }
                pending.cancel_error = Some(error.clone());
                self.capture.ready = Some((pending.seal.request_id(), Err(error)));
                self.capture.pending = Some(pending);
                return;
            }
            self.fail_hosted_capture(pending, error);
            return;
        }
        let Some(draining) = pending.draining.as_mut() else {
            self.capture.pending = Some(pending);
            return;
        };
        match draining.runner.poll_drain(4) {
            Ok(false) => {
                self.capture.pending = Some(pending);
            }
            Err(error) => {
                self.fail_hosted_capture(pending, error);
            }
            Ok(true) => {
                let DrainingCapture { mut packet, runner } = pending.draining.take().unwrap();
                let PacketState::Frozen { device_frame } = packet.state else {
                    unreachable!()
                };
                let native =
                    capture_native_patch(&packet.graph.engine, pending.ticket.native_patch());
                let captured = self.hosted_controller.finish_capture(runner);
                let result = native.and_then(|native| {
                    captured.and_then(|(ack, state)| {
                        HostedProjectCaptureReceipt::new(
                            &pending.ticket,
                            ack,
                            native,
                            state,
                            device_frame,
                        )
                    })
                });
                let mut resumed = false;
                if self.capture_still_current(&pending) {
                    let mut ticket = (*pending.ticket.audio_ticket()).clone();
                    ticket.delta = HostLifecycleDelta::default();
                    if let Err(error) =
                        self.submit_hosted_document_inner(Arc::new(ticket), Some(&mut packet.graph))
                    {
                        self.capture.resume_error = Some(error);
                    } else {
                        resumed = true;
                    }
                }
                if resumed {
                    self.dispose_graph(packet.graph);
                } else {
                    self.retain_capture_recovery(packet.graph);
                }
                self.performance.set_hosted_capture_pending(false);
                self.capture.ready = Some((pending.seal.request_id(), result));
            }
        }
    }

    fn fail_hosted_capture(&mut self, mut pending: PendingCapture, error: String) {
        let mut tried_resume = false;
        if let Some(draining) = pending.draining.take() {
            drop(draining.runner);
            self.hosted_controller.abort_capture(pending.seal.fence());
            let mut packet = draining.packet;
            let mut resumed = false;
            if self.capture_still_current(&pending) && self.pending.is_none() {
                tried_resume = true;
                let mut ticket = (*pending.ticket.audio_ticket()).clone();
                ticket.delta = HostLifecycleDelta::default();
                if let Err(error) =
                    self.submit_hosted_document_inner(Arc::new(ticket), Some(&mut packet.graph))
                {
                    self.capture.resume_error = Some(error);
                } else {
                    resumed = true;
                }
            }
            if resumed {
                self.dispose_graph(packet.graph);
            } else {
                self.retain_capture_recovery(packet.graph);
            }
        }
        // The child may have failed before the returned runner could quiesce.
        // That path already retained its native donor; recover it just as for
        // a failure discovered during drain, without issuing a save receipt.
        if !tried_resume && self.capture_still_current(&pending) && self.pending.is_none() {
            if let Some(mut graph) = self.capture.recovery.take() {
                let mut ticket = (*pending.ticket.audio_ticket()).clone();
                ticket.delta = HostLifecycleDelta::default();
                match self.submit_hosted_document_inner(Arc::new(ticket), Some(&mut graph)) {
                    Ok(()) => self.dispose_graph(graph),
                    Err(error) => {
                        self.capture.resume_error = Some(error);
                        self.retain_capture_recovery(graph);
                    }
                }
            }
        }
        if let Some(control) = &self.hosted_control {
            let _ = control.cancel_capture(pending.seal);
        }
        self.performance.set_hosted_capture_pending(false);
        self.capture.ready = Some((pending.seal.request_id(), Err(error)));
    }

    fn capture_still_current(&self, pending: &PendingCapture) -> bool {
        !self.performance.is_closed()
            && self.submitted_revision == pending.ticket.revision()
            && self.performance.hosted_binding_revision() == pending.ticket.revision()
            && self.performance.native_intent_stamp() == pending.native_intent
            && self
                .hosted_control
                .as_ref()
                .is_some_and(|control| control.fence() == pending.seal.fence())
    }

    fn retain_capture_recovery(&mut self, mut graph: PreparedGraph) {
        if let Some(runner) = graph.hosted.take() {
            if let Some(receipt) = self.hosted_controller.retire(runner) {
                self.last_hosted_retirement = Some(receipt);
            }
        }
        if let Some(old) = self.capture.recovery.replace(graph) {
            self.dispose_graph(old);
        }
    }

    pub(super) fn shutdown_hosted_capture(&mut self) {
        if let Some((_, send)) = self.performance.take_hosted_capture_dispatch() {
            let _ = send.try_send(Err("audio closed before hosted capture dispatch".into()));
        }
        if let Some((_, send)) = self.capture.dispatched.take() {
            let _ = send.try_send(Err("audio closed during hosted capture".into()));
        }
        if let Some(mut pending) = self.capture.pending.take() {
            if let Some(draining) = pending.draining.take() {
                drop(draining.runner);
                self.hosted_controller.abort_capture(pending.seal.fence());
                self.dispose_graph(draining.packet.graph);
            }
            if let Some(control) = &self.hosted_control {
                let _ = control.cancel_capture(pending.seal);
            }
        }
        while let Ok(packet) = self.capture.returns.pop() {
            self.dispose_graph(packet.graph);
        }
        // Callback has already joined. Destroy unread boxed requests before
        // controller process/workspace teardown, including their graph payloads.
        let (empty, reader) = RingBuffer::new(1);
        drop(std::mem::replace(&mut self.capture.requests, empty));
        drop(reader);
        self.capture.ready.take();
        if let Some(graph) = self.capture.recovery.take() {
            self.dispose_graph(graph);
        }
    }
}

fn capture_native_patch(engine: &Engine, authored: &Patch) -> Result<Patch, String> {
    let mut captured = authored.clone();
    for node in &mut captured.nodes {
        if openpatch_core::SavedModuleState::from_node(node).is_some() {
            let handle = engine.resolve_module_state(&node.id)?;
            let state = engine
                .module_saved_state(handle)
                .ok_or("frozen native saved state unavailable")?;
            state.write_node(node);
        }
        for name in openpatch_core::runtime_parameter_names(&node.kind) {
            let handle = engine.resolve_parameter(&node.id, name)?;
            let value = engine
                .parameter_value(handle)
                .ok_or("frozen native parameter unavailable")?;
            node.params.insert((*name).into(), value);
        }
    }
    Ok(captured)
}

#[cfg(test)]
mod tests {
    use super::*;
    include!("audio_hosted_capture_tests.rs");
}
