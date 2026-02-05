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

/// One-writer, many-reader broadcast ring living inside an mmap region.
pub struct BroadcastWriter<T: Copy> {
    _mm: MmapFileMut,
    base: *mut u8,
    mask: u64,
    _pd: PhantomData<T>,
}

pub struct BroadcastReader<T: Copy> {
    _mm: MmapFile,
    base: *const u8,

    read_seq: u64, // local cursor
    mask: u64,
    capacity: u64,
    overruns: u64,
    _pd: PhantomData<T>,
}

impl<T: Copy> BroadcastWriter<T> {
    pub fn create<P: AsRef<Path>>(path: P, cfg: RingConfig) -> io::Result<Self> {
        let bytes = bytes_for_ring::<T>(cfg.capacity);
        let mut mm = MmapFileMut::create_rw(path, bytes)?;
        let base = mm.as_mut_ptr();

        unsafe {
            // header
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

            // slots
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

    #[inline(always)]
    fn header(&self) -> &RingHeader {
        unsafe { &*(self.base as *const RingHeader) }
    }

    #[inline(always)]
    fn slot_mut(&mut self, idx: u64) -> &mut SeqlockSlot<T> {
        let slots = unsafe { self.base.add(size_of::<RingHeader>()) as *mut SeqlockSlot<T> };
        unsafe { &mut *slots.add(idx as usize) }
    }

    /// Publish one item. Single-producer only.
    #[inline(always)]
    pub fn publish(&mut self, item: T) {
        let seq = self.header().write_seq.fetch_add(1, Ordering::Relaxed);
        let idx = seq_to_index(seq, self.mask);
        self.slot_mut(idx).write(item);
    }
}

impl<T: Copy> BroadcastReader<T> {
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let mm = MmapFile::open_ro(path)?;
        let base = mm.as_ptr();

        let h = unsafe { &*(base as *const RingHeader) };
        h.validate::<T>()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        let cap = h.capacity as u64;
        let mask = cap - 1;

        // tail-follow: start at current write seq
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

    #[inline(always)]
    fn header(&self) -> &RingHeader {
        unsafe { &*(self.base as *const RingHeader) }
    }

    #[inline(always)]
    fn slot(&self, idx: u64) -> &SeqlockSlot<T> {
        let slots = unsafe { self.base.add(size_of::<RingHeader>()) as *const SeqlockSlot<T> };
        unsafe { &*slots.add(idx as usize) }
    }

    /// Non-blocking: returns None if no new items.
    #[inline(always)]
    pub fn try_read(&mut self) -> Option<T> {
        let w = self.header().write_seq.load(Ordering::Acquire);

        apply_overrun_policy(w, &mut self.read_seq, self.capacity, &mut self.overruns);

        if self.read_seq >= w {
            return None;
        }

        let idx = seq_to_index(self.read_seq, self.mask);
        let v = self.slot(idx).read();
        self.read_seq += 1;
        Some(v)
    }

    pub fn overruns(&self) -> u64 {
        self.overruns
    }
}
