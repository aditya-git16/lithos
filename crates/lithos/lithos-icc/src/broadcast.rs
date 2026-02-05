//! Single-producer, multi-consumer (SPMC) broadcast ring buffer over shared memory.
//!
//! This module provides a lock-free broadcast mechanism where one writer publishes
//! messages that can be read by multiple independent readers. The ring buffer lives
//! in a memory-mapped file, enabling inter-process communication (IPC).
//!
//! # Design
//! - **Writer**: Holds exclusive write access; publishes items sequentially.
//! - **Readers**: Each reader maintains its own cursor and can independently consume
//!   messages at its own pace. Slow readers that fall behind may experience overruns.
//!
//! # Thread Safety
//! - `BroadcastWriter` is `Send` but NOT `Sync` (single-producer).
//! - `BroadcastReader` is `Send` but NOT `Sync` (each reader is independent).

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
/// Creates and owns a memory-mapped file containing the ring buffer.
/// Only one writer should exist per ring buffer file (single-producer guarantee).
///
/// # Type Parameter
/// - `T`: The element type. Must be `Copy` to allow safe bitwise duplication
///   across process boundaries without requiring serialization.
pub struct BroadcastWriter<T: Copy> {
    /// Owns the mmap lifetime; kept alive but not directly accessed after init.
    _mm: MmapFileMut,
    /// Raw pointer to the start of the mapped region (header location).
    base: *mut u8,
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

        // SAFETY: We just created this mmap region exclusively, so we have sole access.
        // The region is sized correctly for the header + slots.
        unsafe {
            // Initialize the header at the start of the mapped region
            let h = base as *mut RingHeader;
            ptr::write(
                h,
                RingHeader {
                    magic: RING_MAGIC,
                    version: RING_VERSION,
                    capacity: cfg.capacity as u64,
                    elem_size: size_of::<T>() as u64,
                    write_seq: std::sync::atomic::AtomicU64::new(0),
                },
            );

            // Initialize each slot's seqlock to a consistent initial state
            let slots = base.add(size_of::<RingHeader>()) as *mut SeqlockSlot<T>;
            for i in 0..cfg.capacity {
                let s = &mut *slots.add(i);
                s.init();
            }
        }

        Ok(Self {
            _mm: mm,
            base,
            mask: cfg.mask(),
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
        let slots = unsafe { self.base.add(size_of::<RingHeader>()) as *mut SeqlockSlot<T> };
        unsafe { &mut *slots.add(idx as usize) }
    }

    /// Publishes a single item to the ring buffer.
    ///
    /// This is a lock-free operation that:
    /// 1. Atomically increments the write sequence number
    /// 2. Writes the value to the corresponding slot using seqlock protocol
    ///
    /// # Single-Producer Guarantee
    /// This method assumes single-threaded access. Calling from multiple threads
    /// simultaneously will cause data races.
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
        // SAFETY: base points to a validated RingHeader
        unsafe { &*(self.base as *const RingHeader) }
    }

    /// Returns a reference to the slot at the given index.
    #[inline(always)]
    fn slot(&self, idx: u64) -> &SeqlockSlot<T> {
        // SAFETY: idx is always masked to be within capacity bounds
        let slots = unsafe { self.base.add(size_of::<RingHeader>()) as *const SeqlockSlot<T> };
        unsafe { &*slots.add(idx as usize) }
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

        // Adjust read_seq if we've fallen behind (overrun detection/recovery)
        apply_overrun_policy(w, &mut self.read_seq, self.capacity, &mut self.overruns);

        // No new data available
        if self.read_seq >= w {
            return None;
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
