use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicU64, Ordering};

/// Seqock Slot
/// - single write
/// - multiple readers
pub struct SeqlockSlot<T: Copy> {
    seq: AtomicU64,
    data: MaybeUninit<T>,
}

impl<T: Copy> SeqlockSlot<T> {
    #[inline(always)]
    pub fn init(&mut self) {
        self.seq.store(0, Ordering::Relaxed);
    }

    // Publish value to slot , single write only
    #[inline(always)]
    pub fn write(&mut self, value: T) {
        let s0 = self.seq.load(Ordering::Relaxed);
        self.seq.store(s0.wrapping_add(1), Ordering::Release); // odd
        unsafe { self.data.as_mut_ptr().write(value) };
        self.seq.store(s0.wrapping_add(2), Ordering::Release); // even
    }

    /// Read a stable snapshot. Spins until consistent.
    #[inline(always)]
    pub fn read(&self) -> T {
        loop {
            let s1 = self.seq.load(Ordering::Acquire);
            if (s1 & 1) == 1 {
                std::hint::spin_loop();
                continue;
            }

            let v = unsafe { self.data.as_ptr().read() };
            let s2 = self.seq.load(Ordering::Acquire);

            if s1 == s2 && (s2 & 1) == 0 {
                return v;
            }
            std::hint::spin_loop();
        }
    }
}
