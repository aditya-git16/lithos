//! Multi-producer, multi-consumer (MPMC) broadcast ring buffer over shared memory.
//!
//! This module provides a lock-free broadcast mechanism where one or more writers
//! publish messages that can be read by multiple independent readers. The ring buffer
//! lives in a memory-mapped file, enabling inter-process communication (IPC).
//!
//! # Design
//! - **Writers**: Each writer claims a slot atomically via `write_seq.fetch_add(1)`.
//!   Use one `BroadcastWriter` per thread or process; multiple writers open the same
//!   file and each `publish()` gets a unique sequence number, so no two writers
//!   write the same slot concurrently.
//! - **Readers**: Each reader maintains its own cursor and can independently consume
//!   messages at its own pace. Slow readers that fall behind may experience overruns.
//!
//! # Thread Safety
//! - `BroadcastWriter` is `Send` but not `Sync`: do not share one instance across
//!   threads. For multiple producers, open the ring from each thread (or process)
//!   to get a separate `BroadcastWriter` per producer.
//! - `BroadcastReader` is `Send` but not `Sync` (each reader is independent).

use crate::ring::{RingConfig, apply_overrun_policy, seq_to_index};
use crate::seqlock::SeqlockSlot;
use crate::shm_layout::{RING_MAGIC, RING_VERSION, RingHeader, bytes_for_ring};
use lithos_mmap::{MmapFile, MmapFileMut};
use std::io;
use std::marker::PhantomData;
use std::mem::size_of;
use std::path::Path;
use std::ptr;
use std::sync::atomic::Ordering;

/// The writer side of a broadcast ring buffer.
///
/// Creates or opens a memory-mapped file containing the ring buffer.
/// For multiple producers, open the same file from each thread (or process) to get
/// one `BroadcastWriter` per producer; each `publish()` atomically claims a slot.
///
/// # Type Parameter
/// - `T`: The element type. Must be `Copy` to allow safe bitwise duplication
///   across process boundaries without requiring serialization.
pub struct BroadcastWriter<T: Copy> {
    /// Owns the mmap lifetime; kept alive but not directly accessed after init.
    _mm: MmapFileMut,
    /// Raw pointer to the start of the mapped region (header location).
    base: *mut u8,
    /// Cached pointer to the first slot in the ring.
    slots_base: *mut SeqlockSlot<T>,
    /// Bitmask for fast modulo: `index = seq & mask` (capacity must be power of 2).
    mask: u64,
    /// Marker to tie the struct to type `T` without storing a `T`.
    _pd: PhantomData<T>,
}

/// The reader side of a broadcast ring buffer.
///
/// Opens an existing memory-mapped ring buffer file in read-only mode.
/// Multiple readers can open the same file independently and each maintains
/// its own read cursor.
///
/// # Type Parameter
/// - `T`: Must match the element type used by the writer.
pub struct BroadcastReader<T: Copy> {
    /// Owns the mmap lifetime; kept alive but not directly accessed after init.
    _mm: MmapFile,
    /// Raw pointer to the start of the mapped region (read-only).
    base: *const u8,
    /// Cached pointer to the first slot in the ring.
    slots_base: *const SeqlockSlot<T>,
    /// Local read cursor: sequence number of the next item to read.
    read_seq: u64,
    /// Bitmask for fast index calculation.
    mask: u64,
    /// Ring capacity (number of slots).
    capacity: u64,
    /// Count of overrun events (when reader fell too far behind the writer).
    overruns: u64,
    /// Marker to tie the struct to type `T`.
    _pd: PhantomData<T>,
}

impl<T: Copy> BroadcastWriter<T> {
    /// Creates a new broadcast ring buffer at the given file path.
    ///
    /// This initializes the shared memory region with:
    /// - A header containing magic number, version, capacity, and element size
    /// - Pre-initialized seqlock slots for each ring position
    ///
    /// # Arguments
    /// - `path`: File path for the memory-mapped region.
    /// - `cfg`: Ring configuration (capacity must be a power of 2).
    ///
    /// # Errors
    /// Returns an error if file creation or memory mapping fails.
    pub fn create<P: AsRef<Path>>(path: P, cfg: RingConfig) -> io::Result<Self> {
        let bytes = bytes_for_ring::<T>(cfg.capacity);
        let mut mm = MmapFileMut::create_rw(path, bytes)?;
        let base = mm.as_mut_ptr();
        let slots_base = unsafe { base.add(size_of::<RingHeader>()) as *mut SeqlockSlot<T> };

        // We just created this mmap region exclusively, so we have sole access.
        // The region is sized correctly for the header + slots.
        unsafe {
            // Initialize the header at the start of the mapped region
            let h = base as *mut RingHeader;
            ptr::write(
                h,
                RingHeader::new(
                    RING_MAGIC,
                    RING_VERSION,
                    cfg.capacity as u64,
                    size_of::<T>() as u64,
                ),
            );

            // Initialize each slot's seqlock to a consistent initial state
            for i in 0..cfg.capacity {
                let s = &mut *slots_base.add(i);
                s.init();
            }
        }

        Ok(Self {
            _mm: mm,
            base,
            slots_base,
            mask: cfg.mask(),
            _pd: PhantomData,
        })
    }

    /// Opens an existing ring buffer for writing.
    ///
    /// Multiple producers can open the same file (one `BroadcastWriter` per thread or
    /// process); each `publish()` atomically claims a unique slot via `write_seq`.
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let mut mm = MmapFileMut::open_rw(path)?; // need open_rw in lithos_mmap
        let base = mm.as_mut_ptr();
        let slots_base = unsafe { base.add(size_of::<RingHeader>()) as *mut SeqlockSlot<T> };
        let h = unsafe { &*(base as *const RingHeader) };
        let _ = h.validate::<T>();
        let cap = h.capacity;
        Ok(Self {
            _mm: mm,
            base,
            slots_base,
            mask: cap - 1,
            _pd: PhantomData,
        })
    }

    /// Returns a reference to the ring header.
    ///
    /// # Safety
    /// Safe because we own the mmap and the header is always valid after `create()`.
    #[inline(always)]
    fn header(&self) -> &RingHeader {
        // SAFETY: base points to a valid RingHeader that we initialized
        unsafe { &*(self.base as *const RingHeader) }
    }

    /// Returns a mutable reference to the slot at the given index.
    ///
    /// # Safety
    /// The index must be within bounds (enforced by masking in `publish`).
    #[inline(always)]
    fn slot_mut(&mut self, idx: u64) -> &mut SeqlockSlot<T> {
        // SAFETY: idx is always masked to be within capacity bounds
        unsafe { &mut *self.slots_base.add(idx as usize) }
    }

    /// Publishes a single item to the ring buffer.
    ///
    /// This is a lock-free operation that:
    /// 1. Atomically increments the write sequence number (claiming a unique slot)
    /// 2. Writes the value to the corresponding slot using the seqlock protocol
    ///
    /// # Concurrency
    /// Do not call from multiple threads using the same `BroadcastWriter` (this type
    /// is not `Sync`). For multiple producers, use one `BroadcastWriter` per thread
    /// or process, each opened on the same ring file.
    #[inline(always)]
    pub fn publish(&mut self, value: T) {
        // Relaxed ordering is sufficient: the seqlock in the slot provides
        // the necessary synchronization for readers
        let seq = self.header().write_seq.fetch_add(1, Ordering::Relaxed);
        let idx = seq_to_index(seq, self.mask);
        self.slot_mut(idx).write(value);
    }
}

impl<T: Copy> BroadcastReader<T> {
    /// Opens an existing broadcast ring buffer for reading.
    ///
    /// Validates that the file contains a properly formatted ring buffer
    /// with matching element size for type `T`.
    ///
    /// The reader starts at the current write position (tail-follow mode),
    /// meaning it will only see messages published after opening.
    ///
    /// # Arguments
    /// - `path`: Path to an existing ring buffer file created by `BroadcastWriter`.
    ///
    /// # Errors
    /// - File doesn't exist or can't be opened
    /// - Invalid magic number (not a ring buffer file)
    /// - Version mismatch
    /// - Element size mismatch (wrong type `T`)
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let mm = MmapFile::open_ro(path)?;
        let base = mm.as_ptr();
        let slots_base = unsafe { base.add(size_of::<RingHeader>()) as *const SeqlockSlot<T> };

        // SAFETY: We're reading the header to validate it. If the file is corrupted,
        // validate() will catch it.
        let h = unsafe { &*(base as *const RingHeader) };

        // Turbofish `::<T>` passes our type parameter to validate, ensuring
        // the stored elem_size matches size_of::<T>()
        h.validate::<T>()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        let cap = h.capacity as u64;
        let mask = cap - 1;

        // Tail-follow: start reading from the current write position.
        // Acquire ordering ensures we see all writes that happened before this load.
        let read_seq = h.write_seq.load(Ordering::Acquire);

        Ok(Self {
            _mm: mm,
            base,
            slots_base,
            read_seq,
            mask,
            capacity: cap,
            overruns: 0,
            _pd: PhantomData,
        })
    }

    /// Returns a reference to the ring header.
    #[inline(always)]
    fn header(&self) -> &RingHeader {
        // base points to a validated RingHeader
        unsafe { &*(self.base as *const RingHeader) }
    }

    /// Returns a reference to the slot at the given index.
    #[inline(always)]
    fn slot(&self, idx: u64) -> &SeqlockSlot<T> {
        // idx is always masked to be within capacity bounds
        unsafe { &*self.slots_base.add(idx as usize) }
    }

    /// Best-effort prefetch of the next slot likely to be read.
    ///
    /// This is an optimization hint only. It has no semantic effect and may be
    /// ignored by the CPU/OS. On non-x86 architectures this is a no-op.
    #[inline(always)]
    pub fn prefetch_next(&self) {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            let next_idx = seq_to_index(self.read_seq, self.mask);
            let slot_ptr = self.slots_base.add(next_idx as usize) as *const i8;
            core::arch::x86_64::_mm_prefetch(slot_ptr, core::arch::x86_64::_MM_HINT_T0);
        }
    }

    /// Attempts to read the next item from the ring buffer.
    ///
    /// This is a non-blocking operation that returns immediately.
    ///
    /// # Returns
    /// - `Some(T)` if a new item was available and successfully read
    /// - `None` if no new items are available (reader is caught up)
    ///
    /// # Overrun Handling
    /// If the reader has fallen behind and the writer has overwritten unread
    /// slots, the reader's cursor is advanced to the oldest available data.
    /// Check `overruns()` to detect if this has occurred.
    #[inline(always)]
    pub fn try_read(&mut self) -> Option<T> {
        // Acquire ordering ensures we see the most recent write_seq
        let w = self.header().write_seq.load(Ordering::Acquire);

        // No new data available
        if self.read_seq >= w {
            return None;
        }

        // Overruns are rare: only run recovery logic when the reader is clearly behind.
        if w - self.read_seq > self.capacity {
            apply_overrun_policy(w, &mut self.read_seq, self.capacity, &mut self.overruns);
        }

        // Read from the slot corresponding to our current sequence number
        let idx = seq_to_index(self.read_seq, self.mask);
        let v = self.slot(idx).read();
        self.read_seq += 1;
        Some(v)
    }

    /// Returns the total count of overrun events since this reader was opened.
    ///
    /// An overrun occurs when the writer laps the reader, meaning some messages
    /// were lost because the reader couldn't keep up.
    pub fn overruns(&self) -> u64 {
        self.overruns
    }
}
