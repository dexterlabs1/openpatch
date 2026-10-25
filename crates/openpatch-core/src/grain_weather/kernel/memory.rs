//! Original OpenPatch adaptation layer, MIT licensed.
//!
//! The Clouds firmware hands its processor two raw static blocks (118,784 and
//! 65,408 bytes, clouds.cc) and carves recording buffers, effect workspaces
//! and phase-vocoder state out of them with `BufferAllocator`. Several regions
//! alias by design (the pitch shifter reuses the correlator's words, the phase
//! vocoder's phase memory inherits whatever earlier modes left behind), so the
//! adaptation keeps one preallocated little-endian byte image of both blocks
//! and addresses it exactly as the source does. Nothing here allocates after
//! construction.
//!
//! Addresses are byte offsets into a flat image: the large block starts at 0,
//! the small block at `SMALL_BASE`. Accesses that would leave the image are
//! undefined behaviour in the source; they read zero, are dropped on write,
//! and are counted.

use std::cell::Cell;

pub(super) const LARGE_BYTES: usize = 118_784;
pub(super) const SMALL_BYTES: usize = 65_536 - 128;
pub(super) const SMALL_BASE: usize = LARGE_BYTES;
const TOTAL_WORDS: usize = (LARGE_BYTES + SMALL_BYTES) / 4;

#[derive(Debug)]
pub(super) struct Memory {
    words: Box<[u32]>,
    pub(super) guarded: Cell<u64>,
}

impl Clone for Memory {
    fn clone(&self) -> Self {
        Self {
            words: self.words.clone(),
            guarded: Cell::new(self.guarded.get()),
        }
    }
}

impl Memory {
    pub(super) fn new() -> Self {
        Self {
            words: vec![0u32; TOTAL_WORDS].into_boxed_slice(),
            guarded: Cell::new(0),
        }
    }

    /// Zero the image, as the firmware's static storage starts.
    pub(super) fn clear(&mut self) {
        self.words.fill(0);
        self.guarded.set(0);
    }

    pub(super) fn copy_from(&mut self, other: &Memory) {
        self.words.copy_from_slice(&other.words);
        self.guarded.set(other.guarded.get());
    }

    #[inline]
    fn guard(&self) {
        self.guarded.set(self.guarded.get() + 1);
    }

    #[inline]
    pub(super) fn u32(&self, address: usize) -> u32 {
        match self.words.get(address >> 2) {
            Some(&word) => word,
            None => {
                self.guard();
                0
            }
        }
    }

    #[inline]
    pub(super) fn set_u32(&mut self, address: usize, value: u32) {
        match self.words.get_mut(address >> 2) {
            Some(word) => *word = value,
            None => self.guard(),
        }
    }

    #[inline]
    pub(super) fn f32(&self, address: usize) -> f32 {
        f32::from_bits(self.u32(address))
    }

    #[inline]
    pub(super) fn set_f32(&mut self, address: usize, value: f32) {
        self.set_u32(address, value.to_bits());
    }

    #[inline]
    pub(super) fn u16(&self, address: usize) -> u16 {
        (self.u32(address) >> ((address & 2) * 8)) as u16
    }

    #[inline]
    pub(super) fn set_u16(&mut self, address: usize, value: u16) {
        let shift = (address & 2) * 8;
        match self.words.get_mut(address >> 2) {
            Some(word) => *word = (*word & !(0xffff << shift)) | ((value as u32) << shift),
            None => self.guard(),
        }
    }

    #[inline]
    pub(super) fn i16(&self, address: usize) -> i16 {
        self.u16(address) as i16
    }

    #[inline]
    pub(super) fn set_i16(&mut self, address: usize, value: i16) {
        self.set_u16(address, value as u16);
    }

    #[inline]
    pub(super) fn u8(&self, address: usize) -> u8 {
        (self.u32(address) >> ((address & 3) * 8)) as u8
    }

    #[inline]
    pub(super) fn set_u8(&mut self, address: usize, value: u8) {
        let shift = (address & 3) * 8;
        match self.words.get_mut(address >> 2) {
            Some(word) => *word = (*word & !(0xff << shift)) | ((value as u32) << shift),
            None => self.guard(),
        }
    }

    /// Fill `count` 16-bit cells starting at `address` (word-aligned fast path).
    pub(super) fn fill_u16(&mut self, address: usize, count: usize, value: u16) {
        for i in 0..count {
            self.set_u16(address + 2 * i, value);
        }
    }

    pub(super) fn fill_u8(&mut self, address: usize, count: usize, value: u8) {
        for i in 0..count {
            self.set_u8(address + i, value);
        }
    }

    pub(super) fn fill_u32(&mut self, address: usize, count: usize, value: u32) {
        let start = address >> 2;
        match self.words.get_mut(start..start + count) {
            Some(words) if address & 3 == 0 => words.fill(value),
            _ => {
                for i in 0..count {
                    self.set_u32(address + 4 * i, value);
                }
            }
        }
    }

    pub(super) fn store_f32(&mut self, address: usize, source: &[f32]) {
        for (i, value) in source.iter().enumerate() {
            self.set_f32(address + 4 * i, *value);
        }
    }
}

/// `stmlib::BufferAllocator` over the byte image.
#[derive(Clone, Copy, Debug)]
pub(super) struct Allocator {
    next: usize,
    free: usize,
}

impl Allocator {
    pub(super) fn new(address: usize, size: usize) -> Self {
        Self {
            next: address,
            free: size,
        }
    }

    /// Returns the address of `count` elements of `element_size` bytes, or
    /// `None` (the source's NULL) when they do not fit.
    pub(super) fn allocate(&mut self, element_size: usize, count: usize) -> Option<usize> {
        let bytes = element_size * count;
        if bytes <= self.free {
            let start = self.next;
            self.next += bytes;
            self.free -= bytes;
            Some(start)
        } else {
            None
        }
    }

    pub(super) fn free(&self) -> usize {
        self.free
    }
}
