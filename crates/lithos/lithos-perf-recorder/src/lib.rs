//! Lightweight performance stage recorder for Lithos hot paths.
//!
//! When the `record` feature is **off** (production default), `PerfRecorder` is
//! a zero-sized type and every method is an `#[inline(always)]` no-op — zero
//! overhead.
//!
//! When `record` is **on**, each stage gets a pre-allocated `[u64; MAX_SAMPLES]`
//! ring (~20 MB total) and `begin`/`end` pairs store elapsed nanoseconds via
//! `clock_gettime(CLOCK_MONOTONIC)`.

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PerfStage {
    ParseJson = 0,
    ParseNumeric = 1,
    BuildTob = 2,
    TimestampEvent = 3,
    Publish = 4,
    TryRead = 5,
    ProcessEvent = 6,
    PrefetchNext = 7,
    ObsidianTotal = 8,
    OnyxTotal = 9,
}

pub const NUM_STAGES: usize = 10;
pub const MAX_SAMPLES: usize = 262_144; // 256K per stage

// ─── Feature: record ON ─────────────────────────────────────────────────────

#[cfg(feature = "record")]
mod inner {
    use super::*;

    #[cfg(target_os = "macos")]
    #[inline(always)]
    #[allow(deprecated)]
    pub fn now_ns() -> u64 {
        use std::sync::OnceLock;
        static TIMEBASE: OnceLock<(u64, u64)> = OnceLock::new();
        let (numer, denom) = *TIMEBASE.get_or_init(|| {
            let mut info = libc::mach_timebase_info_data_t { numer: 0, denom: 0 };
            let rc = unsafe { libc::mach_timebase_info(&mut info) };
            if rc != 0 || info.denom == 0 {
                (1, 1)
            } else {
                (info.numer as u64, info.denom as u64)
            }
        });
        let t = unsafe { libc::mach_absolute_time() } as u128;
        ((t * numer as u128) / denom as u128) as u64
    }

    #[cfg(not(target_os = "macos"))]
    #[inline(always)]
    pub fn now_ns() -> u64 {
        let mut ts = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        unsafe {
            libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts);
        }
        (ts.tv_sec as u64) * 1_000_000_000 + ts.tv_nsec as u64
    }

    struct StageBuf {
        samples: Box<[u64; MAX_SAMPLES]>,
        count: usize,
        pending: u64,
    }

    impl StageBuf {
        fn new() -> Self {
            Self {
                samples: vec![0u64; MAX_SAMPLES]
                    .into_boxed_slice()
                    .try_into()
                    .unwrap(),
                count: 0,
                pending: 0,
            }
        }
    }

    pub struct PerfRecorder {
        stages: Box<[StageBuf; NUM_STAGES]>,
    }

    impl PerfRecorder {
        pub fn new() -> Self {
            let stages: Vec<StageBuf> = (0..NUM_STAGES).map(|_| StageBuf::new()).collect();
            Self {
                stages: stages.into_boxed_slice().try_into().ok().unwrap(),
            }
        }

        #[inline(always)]
        pub fn begin(&mut self, stage: PerfStage) {
            self.stages[stage as usize].pending = now_ns();
        }

        #[inline(always)]
        pub fn end(&mut self, stage: PerfStage) {
            let buf = &mut self.stages[stage as usize];
            let elapsed = now_ns().saturating_sub(buf.pending);
            if buf.count < MAX_SAMPLES {
                buf.samples[buf.count] = elapsed;
                buf.count += 1;
            }
        }

        #[inline(always)]
        pub fn record(&mut self, stage: PerfStage, duration_ns: u64) {
            let buf = &mut self.stages[stage as usize];
            if buf.count < MAX_SAMPLES {
                buf.samples[buf.count] = duration_ns;
                buf.count += 1;
            }
        }

        pub fn samples(&self, stage: PerfStage) -> &[u64] {
            let buf = &self.stages[stage as usize];
            &buf.samples[..buf.count]
        }

        pub fn count(&self, stage: PerfStage) -> usize {
            self.stages[stage as usize].count
        }

        pub fn drain(&mut self, stage: PerfStage) {
            self.stages[stage as usize].count = 0;
        }

        pub fn reset(&mut self) {
            for buf in self.stages.iter_mut() {
                buf.count = 0;
            }
        }
    }

    impl Default for PerfRecorder {
        fn default() -> Self {
            Self::new()
        }
    }
}

// ─── Feature: record OFF (zero-cost stubs) ──────────────────────────────────

#[cfg(not(feature = "record"))]
mod inner {
    use super::*;

    #[inline(always)]
    pub fn now_ns() -> u64 {
        0
    }

    pub struct PerfRecorder;

    impl PerfRecorder {
        #[inline(always)]
        pub fn new() -> Self {
            Self
        }
        #[inline(always)]
        pub fn begin(&mut self, _stage: PerfStage) {}
        #[inline(always)]
        pub fn end(&mut self, _stage: PerfStage) {}
        #[inline(always)]
        pub fn record(&mut self, _stage: PerfStage, _duration_ns: u64) {}
        #[inline(always)]
        pub fn samples(&self, _stage: PerfStage) -> &[u64] {
            &[]
        }
        #[inline(always)]
        pub fn count(&self, _stage: PerfStage) -> usize {
            0
        }
        #[inline(always)]
        pub fn drain(&mut self, _stage: PerfStage) {}
        #[inline(always)]
        pub fn reset(&mut self) {}
    }

    impl Default for PerfRecorder {
        fn default() -> Self {
            Self
        }
    }
}

pub use inner::{PerfRecorder, now_ns};
