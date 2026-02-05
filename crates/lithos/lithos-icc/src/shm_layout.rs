//! Shared memory layout definitions for the ring buffer.
//!
//! This module defines the binary layout of the memory-mapped ring buffer,
//! including the header structure and size calculations. The layout is
//! designed to be stable across process restarts and compatible with
//! memory-mapped file access.
//!
//! # Memory Layout
//!
//! ```text
//! ┌────────────────────────────────────────────────────────────────┐
//! │                        RingHeader                              │
//! │  ┌──────────┬──────────┬──────────┬──────────┬──────────────┐  │
//! │  │  magic   │ version  │ capacity │ elem_size│  write_seq   │  │
//! │  │  (8B)    │  (8B)    │  (8B)    │  (8B)    │  (8B atomic) │  │
//! │  └──────────┴──────────┴──────────┴──────────┴──────────────┘  │
//! ├────────────────────────────────────────────────────────────────┤
//! │                     SeqlockSlot[0]                             │
//! │  ┌──────────────────┬─────────────────────────────────────┐    │
//! │  │  seq (8B atomic) │  data: T                            │    │
//! │  └──────────────────┴─────────────────────────────────────┘    │
//! ├────────────────────────────────────────────────────────────────┤
//! │                     SeqlockSlot[1]                             │
//! ├────────────────────────────────────────────────────────────────┤
//! │                          ...                                   │
//! ├────────────────────────────────────────────────────────────────┤
//! │                  SeqlockSlot[capacity-1]                       │
//! └────────────────────────────────────────────────────────────────┘
//! ```

use crate::seqlock::SeqlockSlot;
use std::mem::size_of;
use std::sync::atomic::AtomicU64;

/// Magic number identifying a valid ring buffer file.
///
/// ASCII encoding of "LITHOSBU" (Lithos Buffer):
/// `0x4C49_5448_4F53_4255` = "LITHOSBU"
///
/// Used to verify that a memory-mapped file is actually a ring buffer
/// and not some random data.
pub const RING_MAGIC: u64 = 0x4C49_5448_4F53_4255;

/// Current ring buffer format version.
///
/// Increment this when making incompatible changes to the layout.
/// Readers will reject files with mismatched versions.
pub const RING_VERSION: u64 = 1;

/// Header structure at the start of every ring buffer.
///
/// This header is stored at offset 0 in the memory-mapped region and contains
/// all metadata needed for readers to validate and navigate the ring buffer.
///
/// # Representation
/// Uses `#[repr(C)]` to ensure predictable field ordering and alignment,
/// which is critical for cross-process memory-mapped access.
#[repr(C)]
pub struct RingHeader {
    /// Magic number for file type identification. Must equal `RING_MAGIC`.
    pub magic: u64,

    /// Format version for compatibility checking. Must equal `RING_VERSION`.
    pub version: u64,

    /// Number of slots in the ring. Must be a power of 2.
    pub capacity: u64,

    /// Size of each element in bytes. Used to verify type compatibility.
    pub elem_size: u64,

    /// Monotonically increasing count of published items.
    /// Writers increment this atomically; readers use it to detect new data.
    pub write_seq: AtomicU64,
}

impl RingHeader {
    /// Validates the header against expected values.
    ///
    /// This should be called when opening an existing ring buffer to ensure:
    /// - The file is actually a ring buffer (magic check)
    /// - The format version is compatible
    /// - The capacity is valid (power of 2)
    /// - The element size matches the expected type `T`
    ///
    /// # Type Parameter
    /// - `T`: The expected element type. Its `size_of` is compared against `elem_size`.
    ///
    /// # Returns
    /// - `Ok(())` if all checks pass
    /// - `Err(&'static str)` with a description if any check fails
    ///
    /// # Example
    /// ```ignore
    /// let header: &RingHeader = /* ... */;
    /// header.validate::<MyEventType>()?;
    /// ```
    pub fn validate<T: Copy>(&self) -> Result<(), &'static str> {
        if self.magic != RING_MAGIC {
            return Err("Bad magic");
        }
        if self.version != RING_VERSION {
            return Err("Wrong version");
        }
        if (self.capacity as usize).is_power_of_two() == false {
            return Err("Capacity must be power of two");
        }
        if self.elem_size as usize != size_of::<T>() {
            return Err("Element size mismatch");
        }

        Ok(())
    }
}

/// Calculates the total bytes required for a ring buffer region.
///
/// The total size is: `header_size + (capacity × slot_size)`
///
/// # Type Parameter
/// - `T`: The element type stored in each slot.
///
/// # Arguments
/// - `capacity`: Number of slots in the ring.
///
/// # Returns
/// Total bytes needed for the memory-mapped region.
///
/// # Example
/// ```ignore
/// let bytes = bytes_for_ring::<u64>(1024);
/// // bytes = size_of::<RingHeader>() + 1024 * size_of::<SeqlockSlot<u64>>()
/// ```
pub fn bytes_for_ring<T: Copy>(capacity: usize) -> usize {
    size_of::<RingHeader>() + capacity * size_of::<SeqlockSlot<T>>()
}
