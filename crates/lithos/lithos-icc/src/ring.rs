#[derive(Debug, Copy, Clone)]
pub struct RingConfig {
    pub capacity: usize, // must be power of 2
}

impl RingConfig {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity.is_power_of_two(), "Capacity must be power of 2");
        Self { capacity }
    }

    /// Returns the ring buffer bitmask, which is `capacity - 1` (as u64).
    ///
    /// This mask is used to map sequence numbers to valid array indices
    /// with a bitwise AND. Requires that `capacity` is a power of 2.
    ///
    /// # Example
    /// ```
    /// let rc = RingConfig::new(8);
    /// assert_eq!(rc.mask(), 7);
    /// ```
    #[inline(always)]
    pub fn mask(&self) -> u64 {
        (self.capacity as u64) - 1
    }
}

/// Converts a sequence number to a ring buffer array index.
///
/// This function uses a bitwise AND operation with a mask to efficiently map
/// sequence numbers to valid array indices. The mask is `capacity - 1`, which
/// works because the capacity must be a power of 2.
///
///
/// When capacity is a power of 2 (e.g., 8), the mask is `0b111` (binary).
/// The bitwise AND operation `seq & mask` effectively performs modulo arithmetic,
/// wrapping sequence numbers into the valid index range `[0, capacity)`.
///
/// # Examples
///
/// With `capacity = 8` (mask = `0b111` = 7):
/// - `seq = 0` → `0 & 7 = 0`
/// - `seq = 5` → `5 & 7 = 5`
/// - `seq = 8` → `8 & 7 = 0` (wraps around)
/// - `seq = 15` → `15 & 7 = 7`
///
/// # Parameters
///
/// * `seq` - The sequence number to convert (monotonically increasing, can wrap)
/// * `mask` - The bitmask, `capacity - 1` (must be all 1s in binary)
///
/// # Returns
///
/// The array index in the range `[0, capacity)`.
#[inline(always)]
pub fn seq_to_index(seq: u64, mask: u64) -> u64 {
    seq & mask
}

/// Applies an overrun policy when a reader falls too far behind the writer.
///
/// This function handles the case where a writer has produced data faster than
/// a reader can consume it. If the reader is more than `capacity` elements
/// behind, it fast-forwards the reader to prevent reading stale data and to
/// keep the reader within a valid readable window.
///
/// # How it works
///
/// 1. Calculates how far behind the reader is: `behind = write_seq - read_seq`
///    (uses saturating subtraction to avoid underflow)
/// 2. If `behind > capacity`, the reader has fallen too far behind
/// 3. Fast-forwards the reader: `read_seq = write_seq - capacity`
///    This positions the reader at the last readable window (exactly `capacity`
///    elements behind the writer)
/// 4. Tracks skipped elements in `overruns` for telemetry/metrics
///
/// # Example
///
/// With `capacity = 8`, `write_seq = 20`, `read_seq = 5`:
/// - `behind = 20 - 5 = 15`
/// - Since `15 > 8`, policy applies
/// - `overruns += 15 - 8 = 7` (7 elements were skipped)
/// - `read_seq = 20 - 8 = 12` (reader fast-forwards to position 12)
///
/// # Parameters
///
/// * `write_seq` - The current write sequence number (where the writer is)
/// * `read_seq` - Mutable reference to the reader's sequence number (updated if needed)
/// * `capacity` - The ring buffer capacity (must match the mask used in `seq_to_index`)
/// * `overruns` - Mutable reference to a counter tracking skipped elements
///                (used for telemetry/metrics; not strictly necessary for functionality)
#[inline(always)]
pub fn apply_overrun_policy(write_seq: u64, read_seq: &mut u64, capacity: u64, overruns: &mut u64) {
    let behind = write_seq.saturating_sub(*read_seq);
    if behind > capacity {
        *overruns += behind - capacity;
        *read_seq = write_seq - capacity; // jump to last readable window
    }
}
