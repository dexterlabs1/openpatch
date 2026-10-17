//! Bounded audio-side schedule. No process ownership, waits or JSON live here.
use super::control::{ControlFault, HostedControlReader};
use super::{Fault, InputBlock, OutputBlock, OwnedEndpoint, QUANTUM, SAMPLE_RATE};
use openpatch_core::{
    Engine, ExternalFrameError, ExternalFrameIo, PortFrame, ResolvedExternalModule,
};

/// A conservative first profile, pending measured application/device qualification.
pub const TRANSPORT_DELAY_FRAMES: usize = QUANTUM * 3;
pub const MAX_CALLBACK_FRAMES: usize = QUANTUM;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunnerFault {
    UnsupportedProfile,
    Boundary,
    Transport(Fault),
    Sequence,
    FrameOverflow,
    InvalidEvent,
    Control(ControlFault),
}

pub struct HostedRunner {
    endpoint: OwnedEndpoint,
    io: ExternalFrameIo,
    input: Box<InputBlock>,
    output: Box<OutputBlock>,
    frame: u64,
    fault: Option<RunnerFault>,
    awaiting_inputs: bool,
    // The fixed acknowledgment history is prepared off audio. Keep only its
    // pointer in every graph (including native graphs with no hosted runner),
    // rather than inflating callback/adoption stack frames by the entire ring.
    control: Option<Box<HostedControlReader>>,
}

impl HostedRunner {
    /// Control-thread preparation. The endpoint must be retired off the callback.
    pub fn prepare(
        endpoint: OwnedEndpoint,
        handle: ResolvedExternalModule,
    ) -> Result<Self, String> {
        if handle.input_count() != 3 || handle.output_count() != 1 {
            return Err(
                "runner requires the declared three-input/one-output OFFSET boundary".into(),
            );
        }
        let input_mask = handle.input_presence() as u32;
        if input_mask & !7 != 0 || input_mask & 4 == 0 {
            return Err("runner requires the supported OFFSET connection mask".into());
        }
        if endpoint.input_mask() != input_mask
            || endpoint.status() != super::Status::Ready
            || endpoint.submitted_blocks() != 0
            || endpoint.fault().is_some()
        {
            return Err("runner requires a fresh matching ready endpoint".into());
        }
        Ok(Self {
            endpoint,
            io: ExternalFrameIo::new(handle),
            input: Box::new(InputBlock::silence(input_mask)),
            output: Box::default(),
            frame: 0,
            fault: None,
            awaiting_inputs: false,
            control: None,
        })
    }

    pub fn prepare_with_control(
        endpoint: OwnedEndpoint,
        handle: ResolvedExternalModule,
        control: HostedControlReader,
    ) -> Result<Self, String> {
        if endpoint.fence() != control.fence() {
            return Err("hosted event reader belongs to another execution fence".into());
        }
        let mut runner = Self::prepare(endpoint, handle)?;
        runner.control = Some(Box::new(control));
        Ok(runner)
    }

    pub fn frame(&self) -> u64 {
        self.frame
    }
    pub fn fault(&self) -> Option<RunnerFault> {
        self.fault
    }
    pub fn is_priming(&self) -> bool {
        self.endpoint.received_blocks() == 0
    }
    pub fn submitted_blocks(&self) -> u64 {
        self.endpoint.submitted_blocks()
    }

    /// Validate the *whole actual device callback*, before processing a fragment.
    /// Never call this with an internal chunk length to hide a larger callback.
    pub fn validate_callback(&mut self, rate: u32, frames: usize) -> Result<(), RunnerFault> {
        if let Some(fault) = self.fault {
            return Err(fault);
        }
        if rate != SAMPLE_RATE || frames == 0 || frames > MAX_CALLBACK_FRAMES {
            return self.fail(RunnerFault::UnsupportedProfile);
        }
        match self.endpoint.status() {
            super::Status::Ready => {}
            super::Status::Fault(code) => {
                return self.fail(RunnerFault::Transport(Fault::CompanionFault(code)))
            }
            _ => return self.fail(RunnerFault::Transport(Fault::CompanionUnavailable)),
        }
        Ok(())
    }

    /// Append an already-authorized event before processing its target sample.
    /// Equal-sample order is explicit; capacity/order failure latches suspension.
    pub fn queue_parameter(
        &mut self,
        parameter: u16,
        value: f32,
        order: u32,
    ) -> Result<(), RunnerFault> {
        if self.control.is_some() {
            return self.fail(RunnerFault::InvalidEvent);
        }
        self.append_parameter(parameter, value, order)
    }

    fn append_parameter(
        &mut self,
        parameter: u16,
        value: f32,
        order: u32,
    ) -> Result<(), RunnerFault> {
        if let Some(fault) = self.fault {
            return Err(fault);
        }
        let offset = (self.frame % QUANTUM as u64) as u16;
        if self.awaiting_inputs
            || parameter > 1
            || !value.is_finite()
            || !(-1.0..=1.0).contains(&value)
            || self.input.event_count == super::MAX_EVENTS
            || self.input.event_count.checked_sub(1).is_some_and(|last| {
                let event = self.input.events[last];
                (event.sample_offset, event.order) >= (offset, order)
            })
        {
            return self.fail(RunnerFault::InvalidEvent);
        }
        self.input.events[self.input.event_count] = super::ParamEvent {
            sample_offset: offset,
            parameter,
            value,
            order,
        };
        self.input.event_count += 1;
        Ok(())
    }

    /// Called once immediately before the engine processes a frame.
    pub fn before_sample(&mut self) -> Result<&mut ExternalFrameIo, RunnerFault> {
        if let Some(fault) = self.fault {
            return Err(fault);
        }
        if self.awaiting_inputs {
            return self.fail(RunnerFault::Boundary);
        }
        let cursor = (self.frame % QUANTUM as u64) as usize;
        if cursor == 0 && self.frame >= TRANSPORT_DELAY_FRAMES as u64 {
            let expected = (self.frame - TRANSPORT_DELAY_FRAMES as u64) / QUANTUM as u64;
            match self.endpoint.receive_due(&mut self.output) {
                Ok(sequence) if sequence == expected => {}
                Ok(_) => return self.fail(RunnerFault::Sequence),
                Err(fault) => return self.fail(RunnerFault::Transport(fault)),
            }
        }
        if let Some(mut control) = self.control.take() {
            let result = (|| {
                while let Some(event) = control
                    .next_for_sample(self.frame)
                    .map_err(RunnerFault::Control)?
                {
                    self.append_parameter(event.parameter, event.value, event.wire_order)?;
                }
                Ok(())
            })();
            self.control = Some(control);
            if let Err(fault) = result {
                return self.fail(fault);
            }
        }
        self.io.outputs.fill(PortFrame::default());
        if self.frame >= TRANSPORT_DELAY_FRAMES as u64 {
            self.io.outputs[0] = PortFrame {
                channels: self.output.counts[cursor],
                values: self.output.values[cursor],
            };
        }
        self.awaiting_inputs = true;
        Ok(&mut self.io)
    }

    /// Called only after the core has returned canonical inputs consumed at this
    /// frame. Partial input blocks persist across callback boundaries.
    pub fn after_sample(&mut self) -> Result<(), RunnerFault> {
        if let Some(fault) = self.fault {
            return Err(fault);
        }
        if !self.awaiting_inputs {
            return self.fail(RunnerFault::Boundary);
        }
        self.awaiting_inputs = false;
        let cursor = (self.frame % QUANTUM as u64) as usize;
        for port in 0..super::INPUTS {
            let input = self.io.inputs[port];
            self.input.counts[port][cursor] = input.channels;
            if input.channels > super::CHANNELS as u8 {
                return self.fail(RunnerFault::Boundary);
            }
            self.input.values[port][cursor].fill(0.);
            let width = usize::from(input.channels);
            self.input.values[port][cursor][..width].copy_from_slice(&input.values[..width]);
        }
        if cursor + 1 == QUANTUM {
            match self.endpoint.submit(&self.input) {
                Ok(sequence) if sequence == self.frame / QUANTUM as u64 => {}
                Ok(_) => return self.fail(RunnerFault::Sequence),
                Err(fault) => return self.fail(RunnerFault::Transport(fault)),
            }
            self.input.event_count = 0;
            self.input.events.fill(super::ParamEvent::default());
        }
        self.frame = match self.frame.checked_add(1) {
            Some(frame) => frame,
            None => return self.fail(RunnerFault::FrameOverflow),
        };
        if let Some(control) = &mut self.control {
            if let Err(fault) = control.acknowledge(self.endpoint.received_blocks(), self.frame - 1)
            {
                return self.fail(RunnerFault::Control(fault));
            }
        }
        Ok(())
    }

    /// Convenience ordinary-audio frame. The desktop may use the phased APIs
    /// around its existing scope/Listen processing entry point instead.
    pub fn process_frame(&mut self, engine: &mut Engine) -> Result<[f32; 2], RunnerFault> {
        self.process_frame_with_taps_and_listen(engine, &[], &mut [], None)
    }

    /// One atomic core exchange shared by ordinary and desktop scope/Listen paths.
    /// A core rejection latches this runner as well as any host-level suspension.
    pub fn process_frame_with_taps_and_listen<const N: usize>(
        &mut self,
        engine: &mut Engine,
        taps: &[Option<openpatch_core::ResolvedTap>; N],
        samples: &mut [openpatch_core::TapSample; N],
        listen: Option<(
            openpatch_core::ModuleHandle,
            &mut dyn openpatch_core::ListenSink,
        )>,
    ) -> Result<[f32; 2], RunnerFault> {
        if engine.sample_rate() != SAMPLE_RATE {
            return self.fail(RunnerFault::UnsupportedProfile);
        }
        let io = self.before_sample()?;
        let output = engine.process_audio_frame_with_externals_taps_and_listen(
            std::slice::from_mut(io),
            taps,
            samples,
            listen,
        );
        let audio = match output {
            Ok(audio) => audio,
            Err(_error) => return self.fail(RunnerFault::Boundary),
        };
        let priming = self.is_priming();
        self.after_sample()?;
        Ok(if priming { [0.; 2] } else { audio })
    }

    /// Consume only after callback retirement; process waits/draining belong to
    /// the control side. An unsubmitted tail is returned explicitly, never padded.
    pub fn retire(self) -> (OwnedEndpoint, usize, bool, Option<RunnerFault>) {
        (
            self.endpoint,
            (self.frame % QUANTUM as u64) as usize,
            self.awaiting_inputs,
            self.fault,
        )
    }

    fn fail<T>(&mut self, fault: RunnerFault) -> Result<T, RunnerFault> {
        if let Some(control) = &mut self.control {
            control.fail(match fault {
                RunnerFault::Control(fault) => fault,
                _ => ControlFault::Execution,
            });
        }
        self.fault = Some(fault);
        self.output.silence();
        self.io.outputs.fill(PortFrame::default());
        Err(fault)
    }
}

impl From<ExternalFrameError> for RunnerFault {
    fn from(_: ExternalFrameError) -> Self {
        Self::Boundary
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binary_host::{Fence, Region};
    use std::sync::atomic::Ordering;

    fn runner(directory: &tempfile::TempDir) -> (Engine, HostedRunner) {
        let (engine, handle) = super::super::graph::tests::compiled();
        let region = Region::create(
            &directory.path().join("transport"),
            Fence {
                incarnation: 1,
                generation: 1,
                revision: 1,
            },
            4,
            SAMPLE_RATE,
        )
        .unwrap();
        region
            .atomic(super::super::STATUS)
            .store(1, Ordering::Release);
        (
            engine,
            HostedRunner::prepare(region.into_endpoint().unwrap(), handle).unwrap(),
        )
    }

    // Deterministic ownership-controlled peer, not evidence of plugin behavior.
    fn echo_ready(runner: &HostedRunner) {
        use crate::binary_host::{
            HEADER, INPUT_READY, INPUT_VALUES, OUTPUT_COUNTS, OUTPUT_READY, OUTPUT_VALUES, SLOT,
            SLOTS,
        };
        let region = runner.endpoint.region.as_ref();
        for index in 0..SLOTS {
            let slot = HEADER + index * SLOT;
            if region
                .atomic(slot)
                .compare_exchange(INPUT_READY, 2, Ordering::Acquire, Ordering::Relaxed)
                .is_err()
            {
                continue;
            }
            unsafe {
                for frame in 0..QUANTUM {
                    region.write(slot + OUTPUT_COUNTS + frame, 1u8);
                    for lane in 0..super::super::CHANNELS {
                        let value = region.read::<f32>(
                            slot + INPUT_VALUES
                                + ((2 * QUANTUM + frame) * super::super::CHANNELS + lane) * 4,
                        );
                        region.write(
                            slot + OUTPUT_VALUES + (frame * super::super::CHANNELS + lane) * 4,
                            value,
                        );
                    }
                }
            }
            region.atomic(slot).store(OUTPUT_READY, Ordering::Release);
        }
    }

    #[test]
    fn fragmented_callbacks_keep_block_phase_and_declared_delay_without_replaying_events() {
        let dir = tempfile::tempdir().unwrap();
        let (mut engine, mut runner) = runner(&dir);
        let mut audio = Vec::new();
        for length in [17, 111, 1, 127, 63, 65, 3, 125, 128, 128, 128] {
            runner.validate_callback(SAMPLE_RATE, length).unwrap();
            for _ in 0..length {
                if runner.frame() == 7 {
                    runner.queue_parameter(0, 0.25, 10).unwrap();
                    runner.queue_parameter(1, 0.5, 11).unwrap();
                }
                audio.push(runner.process_frame(&mut engine).unwrap()[0]);
            }
            echo_ready(&runner);
        }
        assert!(audio[..TRANSPORT_DELAY_FRAMES + 2]
            .iter()
            .all(|value| *value == 0.));
        assert!(audio[TRANSPORT_DELAY_FRAMES + 2..]
            .iter()
            .all(|value| *value == 0.25));
        assert!(runner.fault().is_none());
        assert_eq!(
            runner.input.event_count, 0,
            "submitted events must not repeat in subsequent blocks"
        );
        assert_eq!(runner.frame(), audio.len() as u64);
    }

    #[test]
    fn missed_deadline_latches_even_if_the_child_publishes_later() {
        let dir = tempfile::tempdir().unwrap();
        let (mut engine, mut runner) = runner(&dir);
        for _ in 0..TRANSPORT_DELAY_FRAMES {
            runner.process_frame(&mut engine).unwrap();
        }
        assert_eq!(
            runner.process_frame(&mut engine),
            Err(RunnerFault::Transport(Fault::MissedDeadline))
        );
        echo_ready(&runner);
        assert_eq!(
            runner.process_frame(&mut engine),
            Err(RunnerFault::Transport(Fault::MissedDeadline))
        );
        let (_, tail, incomplete, fault) = runner.retire();
        assert_eq!(tail, 0);
        assert!(!incomplete);
        assert!(fault.is_some());
    }

    #[test]
    fn core_rejection_latches_the_runner_and_cannot_resume_with_another_engine() {
        let dir = tempfile::tempdir().unwrap();
        let (mut engine, mut runner) = runner(&dir);
        let (mut foreign, _) = super::super::graph::tests::compiled();
        assert_eq!(
            runner.process_frame_with_taps_and_listen(&mut foreign, &[], &mut [], None),
            Err(RunnerFault::Boundary)
        );
        assert_eq!(runner.fault(), Some(RunnerFault::Boundary));
        assert_eq!(
            runner.process_frame(&mut engine),
            Err(RunnerFault::Boundary)
        );
        let (endpoint, tail, incomplete, fault) = runner.retire();
        assert_eq!(endpoint.submitted_blocks(), 0);
        assert_eq!(tail, 0);
        assert!(incomplete);
        assert_eq!(fault, Some(RunnerFault::Boundary));
    }

    #[test]
    fn profile_and_phase_refusals_do_not_submit_or_disguise_unfinished_input() {
        let dir = tempfile::tempdir().unwrap();
        let (mut engine, mut runner) = runner(&dir);
        for _ in 0..13 {
            runner.process_frame(&mut engine).unwrap();
        }
        let (endpoint, tail, incomplete, fault) = runner.retire();
        assert_eq!(tail, 13);
        assert!(!incomplete);
        assert!(fault.is_none());
        assert_eq!(endpoint.submitted_blocks(), 0);
        let dir = tempfile::tempdir().unwrap();
        let (mut engine, mut runner) = self::runner(&dir);
        assert_eq!(
            runner.validate_callback(SAMPLE_RATE, MAX_CALLBACK_FRAMES + 1),
            Err(RunnerFault::UnsupportedProfile)
        );
        assert_eq!(
            runner.process_frame(&mut engine),
            Err(RunnerFault::UnsupportedProfile)
        );
        assert_eq!(runner.submitted_blocks(), 0);
        let dir = tempfile::tempdir().unwrap();
        let (_, mut runner) = self::runner(&dir);
        runner.before_sample().unwrap();
        assert!(matches!(runner.before_sample(), Err(RunnerFault::Boundary)));
    }
}
