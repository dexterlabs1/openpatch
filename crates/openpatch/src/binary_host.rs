//! Bounded v1 companion transport; not yet connected to document/audio playback.
//!
//! One parent endpoint owns payload access. The child follows the ownership
//! protocol in scripts/rack_binary_host/wire.h. Only aligned lock-free u32 words
//! are accessed concurrently. No callback operation waits, allocates or formats.
//! Create, launch, stop/reap, endpoint retirement and Region destruction belong
//! on control threads. A region is never reused for another plan/incarnation.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

pub mod execution;
pub mod graph;
pub mod process;
pub mod runner;

pub const QUANTUM: usize = 128;
pub const INPUTS: usize = 3;
pub const CHANNELS: usize = 16;
pub const MAX_EVENTS: usize = 64;
pub const SAMPLE_RATE: u32 = 48_000;
const HEADER: usize = 128;
const SLOT: usize = 34_368;
const SLOTS: usize = 4;
const BYTES: usize = HEADER + SLOTS * SLOT;
const STATUS: usize = 68;
const STOP: usize = 72;
const ERROR: usize = 76;
const PROCESSED: usize = 80;
const COUNTS: usize = 64;
const INPUT_VALUES: usize = 448;
const EVENT_VALUES: usize = 25_024;
const OUTPUT_COUNTS: usize = 26_048;
const OUTPUT_VALUES: usize = 26_176;
const FREE: u32 = 0;
const INPUT_READY: u32 = 1;
const OUTPUT_READY: u32 = 3;
const READING: u32 = 4;
const WRITING: u32 = 5;
static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);

const _: () = assert!(std::mem::size_of::<AtomicU32>() == 4);
const _: () = assert!(std::mem::align_of::<AtomicU32>() == 4);
const _: () = assert!(cfg!(target_endian = "little"));

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fence {
    pub incarnation: u64,
    pub generation: u64,
    pub revision: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Booting,
    Ready,
    Fault(u32),
    Stopped,
    Invalid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    CompanionUnavailable,
    CompanionFault(u32),
    InvalidInput,
    QueueFull,
    MissedDeadline,
    InvalidOutput,
    StaleOutput,
    SequenceExhausted,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ParamEvent {
    pub sample_offset: u16,
    pub parameter: u16,
    pub value: f32,
    pub order: u32,
}

pub struct InputBlock {
    pub counts: [[u8; QUANTUM]; INPUTS],
    pub values: [[[f32; CHANNELS]; QUANTUM]; INPUTS],
    pub events: [ParamEvent; MAX_EVENTS],
    pub event_count: usize,
}

impl InputBlock {
    pub fn silence(input_mask: u32) -> Self {
        let mut counts = [[0; QUANTUM]; INPUTS];
        for (port, values) in counts.iter_mut().enumerate() {
            if input_mask & (1 << port) != 0 {
                values.fill(1);
            }
        }
        Self {
            counts,
            values: [[[0.0; CHANNELS]; QUANTUM]; INPUTS],
            events: [ParamEvent::default(); MAX_EVENTS],
            event_count: 0,
        }
    }

    fn valid(&self, input_mask: u32) -> bool {
        if self.event_count > MAX_EVENTS {
            return false;
        }
        for port in 0..INPUTS {
            for frame in 0..QUANTUM {
                let width = self.counts[port][frame] as usize;
                let connected = input_mask & (1 << port) != 0;
                if width > CHANNELS || (connected && width == 0) || (!connected && width != 0) {
                    return false;
                }
                for (channel, value) in self.values[port][frame].iter().enumerate() {
                    if !value.is_finite() || (channel >= width && *value != 0.0) {
                        return false;
                    }
                }
            }
        }
        let mut previous = None;
        for event in &self.events[..self.event_count] {
            let key = (event.sample_offset, event.order);
            if event.sample_offset as usize >= QUANTUM
                || event.parameter > 1
                || !event.value.is_finite()
                || !(-1.0..=1.0).contains(&event.value)
                || previous.is_some_and(|p| p >= key)
            {
                return false;
            }
            previous = Some(key);
        }
        true
    }
}

#[derive(Debug)]
pub struct OutputBlock {
    pub counts: [u8; QUANTUM],
    pub values: [[f32; CHANNELS]; QUANTUM],
}

impl Default for OutputBlock {
    fn default() -> Self {
        Self {
            counts: [0; QUANTUM],
            values: [[0.0; CHANNELS]; QUANTUM],
        }
    }
}

impl OutputBlock {
    pub fn silence(&mut self) {
        self.counts.fill(0);
        self.values.fill([0.0; CHANNELS]);
    }
}

pub struct Region {
    base: NonNull<u8>,
    _file: File,
    fence: Fence,
    input_mask: u32,
    endpoint_taken: bool,
    owner_id: u64,
}

// A mapping remains valid when ownership moves between parent threads. Region
// is deliberately not Sync: only one parent endpoint may touch its payloads.
unsafe impl Send for Region {}

impl AsRef<Region> for Region {
    fn as_ref(&self) -> &Region {
        self
    }
}

/// Independent header-only control mapping. It cannot access slot payloads or
/// construct endpoints. The process owner keeps this on its control thread while
/// the unique OwnedEndpoint moves into a prepared/retired audio plan.
pub struct ControlView {
    base: NonNull<u8>,
    _file: File,
    owner_id: u64,
}
// The only accessible shared bytes are aligned initialized atomic u32 words.
unsafe impl Send for ControlView {}
unsafe impl Sync for ControlView {}

impl ControlView {
    fn atomic(&self, offset: usize) -> &AtomicU32 {
        unsafe { &*self.base.as_ptr().add(offset).cast::<AtomicU32>() }
    }
    pub fn status(&self) -> Status {
        match self.atomic(STATUS).load(Ordering::Acquire) {
            0 => Status::Booting,
            1 => Status::Ready,
            2 => Status::Fault(self.atomic(ERROR).load(Ordering::Acquire)),
            3 => Status::Stopped,
            _ => Status::Invalid,
        }
    }
    pub fn request_stop(&self) {
        self.atomic(STOP).store(1, Ordering::Release);
    }
    pub fn processed_blocks(&self) -> u32 {
        self.atomic(PROCESSED).load(Ordering::Acquire)
    }
    /// Bind capture to this exact mapping even if external fences were reused.
    pub fn owns_endpoint(&self, endpoint: &OwnedEndpoint) -> bool {
        self.owner_id == endpoint.region.owner_id
    }
}
impl Drop for ControlView {
    fn drop(&mut self) {
        unsafe {
            libc::munmap(self.base.as_ptr().cast(), HEADER);
        }
    }
}

impl Region {
    /// Control-thread setup only. Creates a new private file; never replaces an
    /// existing file. Caller must stop/reap the child before dropping the region.
    pub fn create(path: &Path, fence: Fence, input_mask: u32, rate: u32) -> io::Result<Self> {
        if fence.incarnation == 0
            || fence.generation == 0
            || fence.revision == 0
            || input_mask & !7 != 0
            || input_mask & 4 == 0
            || rate != SAMPLE_RATE
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unsupported host configuration",
            ));
        }
        let mut owner_id = NEXT_OWNER.load(Ordering::Relaxed);
        loop {
            let next = owner_id
                .checked_add(1)
                .ok_or_else(|| io::Error::other("host mapping identities exhausted"))?;
            match NEXT_OWNER.compare_exchange_weak(
                owner_id,
                next,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(observed) => owner_id = observed,
            }
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?;
        file.set_len(BYTES as u64)?;
        // SAFETY: new file has exactly BYTES bytes and stays open. The mapping
        // owns this address until Drop; MAP_SHARED supplies the child transport.
        let raw = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                BYTES,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                file.as_raw_fd(),
                0,
            )
        };
        if raw == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        let base = NonNull::new(raw.cast::<u8>()).expect("mmap returned null mapping");
        let region = Self {
            base,
            _file: file,
            fence,
            input_mask,
            endpoint_taken: false,
            owner_id,
        };
        // SAFETY: no child has been launched; all bytes are exclusively owned.
        unsafe {
            std::ptr::write_bytes(region.base.as_ptr(), 0, BYTES);
            std::ptr::copy_nonoverlapping(b"OPHST001".as_ptr(), region.base.as_ptr(), 8);
            for (offset, value) in [
                (8, 1),
                (12, BYTES as u32),
                (16, QUANTUM as u32),
                (20, rate),
                (24, SLOTS as u32),
                (28, MAX_EVENTS as u32),
                (56, INPUTS as u32),
                (60, 1),
                (64, CHANNELS as u32),
                (84, input_mask),
            ] {
                region.write(offset, value);
            }
            region.write(32, fence.incarnation);
            region.write(40, fence.generation);
            region.write(48, fence.revision);
            for offset in [STATUS, STOP, ERROR, PROCESSED] {
                std::ptr::write(
                    region.base.as_ptr().add(offset).cast::<AtomicU32>(),
                    AtomicU32::new(0),
                );
            }
            for slot in 0..SLOTS {
                std::ptr::write(
                    region
                        .base
                        .as_ptr()
                        .add(HEADER + slot * SLOT)
                        .cast::<AtomicU32>(),
                    AtomicU32::new(FREE),
                );
            }
        }
        Ok(region)
    }

    fn atomic(&self, offset: usize) -> &AtomicU32 {
        // SAFETY: callers use only initialized, aligned shared atomic words.
        unsafe { &*self.base.as_ptr().add(offset).cast::<AtomicU32>() }
    }

    unsafe fn write<T: Copy>(&self, offset: usize, value: T) {
        // SAFETY: caller owns this non-atomic payload or unlaunched header.
        unsafe {
            std::ptr::write_unaligned(self.base.as_ptr().add(offset).cast::<T>(), value);
        }
    }

    unsafe fn read<T: Copy>(&self, offset: usize) -> T {
        // SAFETY: caller owns this payload; scalar reads never form references.
        unsafe { std::ptr::read_unaligned(self.base.as_ptr().add(offset).cast::<T>()) }
    }

    pub fn status(&self) -> Status {
        match self.atomic(STATUS).load(Ordering::Acquire) {
            0 => Status::Booting,
            1 => Status::Ready,
            2 => Status::Fault(self.atomic(ERROR).load(Ordering::Acquire)),
            3 => Status::Stopped,
            _ => Status::Invalid,
        }
    }

    pub fn request_stop(&self) {
        self.atomic(STOP).store(1, Ordering::Release);
    }
    pub fn processed_blocks(&self) -> u32 {
        self.atomic(PROCESSED).load(Ordering::Acquire)
    }

    /// Control-thread preparation only; maps just the existing header using an
    /// independently owned descriptor. No clone of payload ownership is exposed.
    pub fn control_view(&self) -> io::Result<ControlView> {
        let file = self._file.try_clone()?;
        let raw = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                HEADER,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                file.as_raw_fd(),
                0,
            )
        };
        if raw == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        let base = NonNull::new(raw.cast::<u8>()).expect("mmap returned null control mapping");
        Ok(ControlView {
            base,
            _file: file,
            owner_id: self.owner_id,
        })
    }

    /// Move the sole payload owner into an audio plan, with no self-reference to
    /// the control process owner. Retire/drop it on a control thread, not inside
    /// the callback. A previously borrowed endpoint cannot be reset this way.
    pub fn into_endpoint(mut self) -> Option<OwnedEndpoint> {
        if self.endpoint_taken {
            return None;
        }
        self.endpoint_taken = true;
        Some(TransportEndpoint {
            region: self,
            next_submit: 0,
            next_receive: 0,
            fault: None,
        })
    }

    /// One endpoint for the entire mapping lifetime; dropping it does not reset
    /// sequence ownership or allow another endpoint to replay old slot contents.
    pub fn take_endpoint(&mut self) -> Option<Endpoint<'_>> {
        if self.endpoint_taken {
            return None;
        }
        self.endpoint_taken = true;
        Some(Endpoint {
            region: self,
            next_submit: 0,
            next_receive: 0,
            fault: None,
        })
    }
}

impl Drop for Region {
    fn drop(&mut self) {
        // SAFETY: exactly this owner unmaps the original address and length.
        unsafe {
            libc::munmap(self.base.as_ptr().cast(), BYTES);
        }
    }
}

pub type Endpoint<'a> = TransportEndpoint<&'a mut Region>;
pub type OwnedEndpoint = TransportEndpoint<Region>;

pub struct TransportEndpoint<R> {
    region: R,
    next_submit: u64,
    next_receive: u64,
    fault: Option<Fault>,
}

impl<R: AsRef<Region>> TransportEndpoint<R> {
    pub fn input_mask(&self) -> u32 {
        self.region.as_ref().input_mask
    }
    pub fn fence(&self) -> Fence {
        self.region.as_ref().fence
    }
    pub fn submitted_blocks(&self) -> u64 {
        self.next_submit
    }
    pub fn received_blocks(&self) -> u64 {
        self.next_receive
    }
    pub fn is_drained(&self) -> bool {
        self.next_submit == self.next_receive
    }
    pub fn fault(&self) -> Option<Fault> {
        self.fault
    }
    pub fn status(&self) -> Status {
        self.region.as_ref().status()
    }
    pub fn request_stop(&self) {
        self.region.as_ref().request_stop();
    }
    pub fn processed_blocks(&self) -> u32 {
        self.region.as_ref().processed_blocks()
    }
    pub fn is_output_ready(&self) -> bool {
        self.next_receive < self.next_submit
            && self
                .region
                .as_ref()
                .atomic(HEADER + (self.next_receive % SLOTS as u64) as usize * SLOT)
                .load(Ordering::Acquire)
                == OUTPUT_READY
    }

    fn fail<T>(&mut self, fault: Fault) -> Result<T, Fault> {
        let fault = *self.fault.get_or_insert(fault);
        Err(fault)
    }

    fn live(&mut self) -> Result<(), Fault> {
        if let Some(fault) = self.fault {
            return Err(fault);
        }
        match self.region.as_ref().status() {
            Status::Ready => Ok(()),
            Status::Fault(code) => self.fail(Fault::CompanionFault(code)),
            _ => self.fail(Fault::CompanionUnavailable),
        }
    }

    /// No allocation, lock, retry or IPC wait. Queue exhaustion is a latched
    /// execution failure, never permission to overwrite a child-owned slot.
    pub fn submit(&mut self, input: &InputBlock) -> Result<u64, Fault> {
        self.live()?;
        if !input.valid(self.region.as_ref().input_mask) {
            return self.fail(Fault::InvalidInput);
        }
        let sequence = self.next_submit;
        let Some(next) = sequence.checked_add(1) else {
            return self.fail(Fault::SequenceExhausted);
        };
        let slot = HEADER + (sequence % SLOTS as u64) as usize * SLOT;
        if self
            .region
            .as_ref()
            .atomic(slot)
            .compare_exchange(FREE, WRITING, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return self.fail(Fault::QueueFull);
        }
        // SAFETY: successful ownership claim excludes child/payload readers.
        unsafe {
            std::ptr::write_bytes(
                self.region.as_ref().base.as_ptr().add(slot + 4),
                0,
                SLOT - 4,
            );
            self.region.as_ref().write(slot + 8, sequence);
            self.region
                .as_ref()
                .write(slot + 16, self.region.as_ref().fence.incarnation);
            self.region
                .as_ref()
                .write(slot + 24, self.region.as_ref().fence.generation);
            self.region
                .as_ref()
                .write(slot + 32, self.region.as_ref().fence.revision);
            self.region
                .as_ref()
                .write(slot + 40, input.event_count as u32);
            std::ptr::copy_nonoverlapping(
                input.counts.as_ptr().cast::<u8>(),
                self.region.as_ref().base.as_ptr().add(slot + COUNTS),
                INPUTS * QUANTUM,
            );
            std::ptr::copy_nonoverlapping(
                input.values.as_ptr().cast::<u8>(),
                self.region.as_ref().base.as_ptr().add(slot + INPUT_VALUES),
                INPUTS * QUANTUM * CHANNELS * 4,
            );
            for (index, event) in input.events[..input.event_count].iter().enumerate() {
                let offset = slot + EVENT_VALUES + index * 16;
                self.region.as_ref().write(offset, event.sample_offset);
                self.region.as_ref().write(offset + 2, event.parameter);
                self.region.as_ref().write(offset + 4, event.value);
                self.region.as_ref().write(offset + 8, event.order);
            }
        }
        self.region
            .as_ref()
            .atomic(slot)
            .store(INPUT_READY, Ordering::Release);
        self.next_submit = next;
        Ok(sequence)
    }

    /// Nonblocking observation, useful for off-callback preparation/testing.
    /// Not-ready is silent. At an audio deadline use receive_due instead.
    pub fn try_receive(&mut self, output: &mut OutputBlock) -> Result<Option<u64>, Fault> {
        output.silence();
        self.live()?;
        if self.next_receive == self.next_submit {
            return Ok(None);
        }
        let sequence = self.next_receive;
        let slot = HEADER + (sequence % SLOTS as u64) as usize * SLOT;
        if self
            .region
            .as_ref()
            .atomic(slot)
            .compare_exchange(OUTPUT_READY, READING, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return Ok(None);
        }
        // SAFETY: child published all fields and cannot access them again until
        // the parent publishes FREE after copying/validating the entire result.
        let result = unsafe {
            let fence = self.region.as_ref().fence;
            if self.region.as_ref().read::<u64>(slot + 8) != sequence
                || self.region.as_ref().read::<u64>(slot + 16) != fence.incarnation
                || self.region.as_ref().read::<u64>(slot + 24) != fence.generation
                || self.region.as_ref().read::<u64>(slot + 32) != fence.revision
            {
                Err(Fault::StaleOutput)
            } else if self.region.as_ref().read::<u32>(slot + 4) != 0
                || (44..64).any(|offset| self.region.as_ref().read::<u8>(slot + offset) != 0)
            {
                Err(Fault::InvalidOutput)
            } else {
                std::ptr::copy_nonoverlapping(
                    self.region.as_ref().base.as_ptr().add(slot + OUTPUT_COUNTS),
                    output.counts.as_mut_ptr(),
                    QUANTUM,
                );
                std::ptr::copy_nonoverlapping(
                    self.region.as_ref().base.as_ptr().add(slot + OUTPUT_VALUES),
                    output.values.as_mut_ptr().cast::<u8>(),
                    QUANTUM * CHANNELS * 4,
                );
                let valid = output
                    .counts
                    .iter()
                    .zip(&output.values)
                    .all(|(&width, values)| {
                        width as usize <= CHANNELS
                            && values.iter().enumerate().all(|(channel, value)| {
                                value.is_finite() && (channel < width as usize || *value == 0.0)
                            })
                    });
                if valid {
                    Ok(())
                } else {
                    Err(Fault::InvalidOutput)
                }
            }
        };
        self.region
            .as_ref()
            .atomic(slot)
            .store(FREE, Ordering::Release);
        if let Err(fault) = result {
            output.silence();
            return self.fail(fault);
        }
        self.next_receive += 1;
        Ok(Some(sequence))
    }

    /// Called at the declared quantum deadline. Never waits or repeats old CV;
    /// failure is latched until the control owner creates a fresh incarnation.
    pub fn receive_due(&mut self, output: &mut OutputBlock) -> Result<u64, Fault> {
        match self.try_receive(output)? {
            Some(sequence) => Ok(sequence),
            None => self.fail(Fault::MissedDeadline),
        }
    }
}

#[cfg(test)]
mod tests;
