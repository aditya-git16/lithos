//! Ring buffer configuration and index arithmetic utilities.
//!
//! This module provides the foundational primitives for power-of-two ring buffers:
//! - Configuration with capacity validation
//! - Efficient sequence-to-index mapping using bitmasks
//! - Overrun detection and recovery for slow readers

/// Configuration for a ring buffer.
///
/// The capacity must always be a power of 2, enabling efficient index
/// calculations via bitmasking instead of expensive modulo operations.
#[derive(Debug, Copy, Clone)]
pub struct RingConfig {
    /// Number of slots in the ring. Must be a power of 2.
    pub capacity: usize,
}

impl RingConfig {
    /// Creates a new ring configuration with the specified capacity.
    ///
    /// # Panics
    /// Panics if `capacity` is not a power of 2.
    ///
    /// # Example
    /// ```
    /// use lithos_icc::RingConfig;
    /// let cfg = RingConfig::new(1024); // OK: 1024 = 2^10
    /// // RingConfig::new(1000);        // Would panic: not a power of 2
    /// ```
    pub fn new(capacity: usize) -> Self {
        assert!(capacity.is_power_of_two(), "Capacity must be power of 2");
        Self { capacity }
    }

    /// Returns the bitmask for efficient index calculation.
    ///
    /// The mask is `capacity - 1`, which has all lower bits set to 1.
    /// Using `seq & mask` is equivalent to `seq % capacity` but much faster.
    ///
    /// # Example
    /// ```
    /// use lithos_icc::RingConfig;
    /// let cfg = RingConfig::new(8);
    /// assert_eq!(cfg.mask(), 7);  // 0b111 in binary
    /// ```
    #[inline(always)]
    pub fn mask(&self) -> u64 {
        (self.capacity as u64) - 1
    }
}

/// Converts a sequence number to a ring buffer array index.
///
/// Uses bitwise AND with a mask for O(1) index calculation.
/// This works because the capacity is always a power of 2.
///
/// # How It Works
///
/// When capacity is a power of 2 (e.g., 8), the mask is `capacity - 1` = `0b111`.
/// The bitwise AND operation `seq & mask` extracts only the lower bits,
/// effectively performing `seq % capacity` without division.
///
/// # Examples
///
/// With `capacity = 8` (mask = 7 = `0b111`):
/// ```text
/// seq =  0 → 0 & 7 = 0
/// seq =  5 → 5 & 7 = 5
/// seq =  8 → 8 & 7 = 0  (wraps around)
/// seq = 15 → 15 & 7 = 7
/// seq = 16 → 16 & 7 = 0  (wraps again)
/// ```
///
/// # Arguments
/// - `seq`: Monotonically increasing sequence number (may wrap at u64::MAX)
/// - `mask`: Bitmask equal to `capacity - 1`
///
/// # Returns
/// Array index in the range `[0, capacity)`.
#[inline(always)]
pub fn seq_to_index(seq: u64, mask: u64) -> u64 {
    seq & mask
}

/// Detects and recovers from reader overruns.
///
/// An overrun occurs when the writer has lapped the reader—i.e., the reader
/// is so far behind that the writer has overwritten data the reader hasn't
/// consumed yet.
///
/// # Recovery Strategy
///
/// When an overrun is detected, the reader is "fast-forwarded" to the oldest
/// valid data (exactly `capacity` elements behind the writer). This ensures
/// the reader always sees consistent data, though some messages are lost.
///
/// # Algorithm
///
/// 1. Calculate lag: `behind = write_seq - read_seq`
/// 2. If `behind > capacity`, overrun has occurred:
///    - Count skipped messages: `overruns += behind - capacity`
///    - Fast-forward reader: `read_seq = write_seq - capacity`
///
/// # Example
///
/// ```text
/// capacity = 8, write_seq = 20, read_seq = 5
///
/// behind = 20 - 5 = 15
/// 15 > 8, so overrun detected!
/// skipped = 15 - 8 = 7 messages lost
/// read_seq = 20 - 8 = 12 (reader jumps to oldest available)
/// ```
///
/// # Arguments
/// - `write_seq`: Current writer position (sequence number)
/// - `read_seq`: Reader's current position (mutated if overrun detected)
/// - `capacity`: Ring buffer capacity
/// - `overruns`: Counter for lost messages (mutated to track total overruns)
#[inline(always)]
pub fn apply_overrun_policy(write_seq: u64, read_seq: &mut u64, capacity: u64, overruns: &mut u64) {
    // saturating_sub prevents underflow if read_seq somehow exceeds write_seq
    let behind = write_seq.saturating_sub(*read_seq);
    if behind > capacity {
        // Track how many messages were lost
        *overruns += behind - capacity;
        // Jump to the oldest still-valid position
        *read_seq = write_seq - capacity;
    }
}
