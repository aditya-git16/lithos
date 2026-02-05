use crate::seqlock::SeqlockSlot;
use std::mem::size_of;
use std::sync::atomic::AtomicU64;

pub const RING_MAGIC: u64 = 0x4C49_5448_4F53_4255;
pub const RING_VERSION: u64 = 1;

#[repr(C)]
pub struct RingHeader{
    pub magic: u64,
    pub version: u64,
    pub capacity: u64,
    pub elem_size: u64,
    pub write_seq: AtomicU64, // monotonic count of pubished items
}

impl RingHeader {
    pub fn validate<T : Copy>(&self) -> Result<(), &'static str> {
        if self.magic != RING_MAGIC {
            return Err("Bad magic");
        }
        if self.version != RING_VERSION {
            return Err("Wrong version")
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

/// Total bytes required for the shm (mmap) region
pub fn bytes_for_ring<T: Copy>(capacity: usize) -> usize {
    size_of::<RingHeader>() + capacity * size_of::<SeqlockSlot<T>>()
}