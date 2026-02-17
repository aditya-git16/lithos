//! Seqlock (sequence lock) implementation for lock-free single-writer, multi-reader access.
//!
//! A seqlock is a synchronization primitive that allows one writer and multiple readers
//! to access shared data without blocking. The writer increments a sequence number before
//! and after writing; readers detect concurrent writes by checking if the sequence changed.
//!
//! # Protocol
//!
//! **Writer:**
//! 1. Increment seq to odd (signals "write in progress")
//! 2. Write data
//! 3. Increment seq to even (signals "write complete")
//!
//! **Reader:**
//! 1. Read seq; if odd, spin (write in progress)
//! 2. Copy data
//! 3. Read seq again; if changed, retry from step 1
//! 4. Return data (guaranteed consistent)
//!
//! # Trade-offs
//!
//! - **Pros**: Lock-free, no blocking, excellent for read-heavy workloads
//! - **Cons**: Readers may spin during writes, requires `Copy` data

use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicU64, Ordering};

/// A slot protected by a sequence lock.
///
/// Provides lock-free synchronization for a single writer and multiple readers.
/// The data type `T` must be `Copy` to allow safe bitwise reads without worrying
/// about partial/torn reads of complex types.
///
/// # Memory Layout
///
/// ```text
/// ┌─────────────────────────────────────┐
/// │  seq: AtomicU64  │  data: T         │
/// │  (8 bytes)       │  (size_of::<T>)  │
/// └─────────────────────────────────────┘
/// ```
///
/// # Sequence Number Semantics
///
/// - **Even**: Data is stable, safe to read
/// - **Odd**: Write in progress, readers must wait
#[repr(C, align(64))]
pub struct SeqlockSlot<T: Copy> {
    /// Sequence counter: odd = write in progress, even = stable.
    seq: AtomicU64,
    /// The actual data. Uses `MaybeUninit` because we initialize it lazily.
    data: MaybeUninit<T>,
}

impl<T: Copy> SeqlockSlot<T> {
    /// Initializes the slot to a clean state.
    ///
    /// Sets the sequence number to 0 (even = stable).
    /// The data field is left uninitialized until the first `write()`.
    #[inline(always)]
    pub fn init(&mut self) {
        self.seq.store(0, Ordering::Relaxed);
    }

    /// Writes a value to the slot using the seqlock protocol.
    ///
    /// # Protocol Steps
    /// 1. Load current sequence number
    /// 2. Store `seq + 1` (odd) with Release ordering → signals "write starting"
    /// 3. Write the data
    /// 4. Store `seq + 2` (even) with Release ordering → signals "write complete"
    ///
    /// # Single-Writer Per Slot
    /// This method is not thread-safe for multiple writers on the *same* slot.
    /// Only one thread should call `write()` on a given slot at a time. In the
    /// broadcast ring, multiple producers are safe because each `publish()` gets
    /// a unique sequence number and thus a unique slot, so no two writers touch
    /// the same slot concurrently.
    ///
    /// # Memory Ordering
    /// - `Release` on seq stores ensures the data write is visible to readers
    ///   who observe the updated sequence number.
    #[inline(always)]
    pub fn write(&mut self, value: T) {
        let s0 = self.seq.load(Ordering::Relaxed);
        // Mark write-in-progress (odd sequence number)
        self.seq.store(s0.wrapping_add(1), Ordering::Release);
        // SAFETY: We have exclusive write access; data pointer is valid
        unsafe { self.data.as_mut_ptr().write(value) };
        // Mark write-complete (even sequence number)
        self.seq.store(s0.wrapping_add(2), Ordering::Release);
    }

    /// Reads a consistent snapshot of the data, spinning if necessary.
    ///
    /// This method will spin-wait until it can read a consistent value—i.e.,
    /// a value that wasn't being modified during the read.
    ///
    /// # Protocol Steps
    /// 1. Load seq with Acquire ordering
    /// 2. If seq is odd (write in progress), spin and retry
    /// 3. Copy the data
    /// 4. Load seq again with Acquire ordering
    /// 5. If seq changed or is odd, the read was torn—retry from step 1
    /// 6. Return the consistent data
    ///
    /// # Memory Ordering
    /// - `Acquire` on seq loads synchronizes with the writer's `Release` stores,
    ///   ensuring we see the complete data written before the sequence update.
    ///
    /// # Blocking Behavior
    /// This method spins (busy-waits) if a write is in progress. For very
    /// long writes, this could waste CPU cycles. Use `spin_loop` hint to
    /// be friendly to hyperthreading / power management.
    #[inline(always)]
    pub fn read(&self) -> T {
        loop {
            // Step 1: Read sequence number
            let s1 = self.seq.load(Ordering::Acquire);

            // Step 2: If odd, a write is in progress—spin and retry
            if (s1 & 1) == 1 {
                std::hint::spin_loop();
                continue;
            }

            // Step 3: Copy the data (may be torn if write starts now)
            // SAFETY: Data is initialized after first write; we verify consistency below
            let v = unsafe { self.data.as_ptr().read() };

            // Step 4-5: Verify no write occurred during our read
            let s2 = self.seq.load(Ordering::Acquire);
            if s1 == s2 && (s2 & 1) == 0 {
                // Consistent read: seq didn't change and is still even
                return v;
            }

            // Read was torn (seq changed)—retry
            std::hint::spin_loop();
        }
    }
}
