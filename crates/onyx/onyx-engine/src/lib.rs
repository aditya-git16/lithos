use lithos_events::TopOfBook;
use lithos_icc::BroadcastReader;
use onyx_core::MarketStateManager;
use std::path::Path;

#[cfg(feature = "perf")]
use lithos_perf_recorder::{PerfRecorder, PerfStage};

pub struct OnyxEngine {
    pub market_state_manager: MarketStateManager,
    pub reader: BroadcastReader<TopOfBook>,
    #[cfg(feature = "perf")]
    pub perf: PerfRecorder,
}

impl OnyxEngine {
    pub fn new<P: AsRef<Path>>(path: P) -> std::io::Result<Self> {
        let market_state_manager = MarketStateManager::new();
        let reader = BroadcastReader::<TopOfBook>::open(path)?;
        Ok(OnyxEngine {
            market_state_manager,
            reader,
            #[cfg(feature = "perf")]
            perf: PerfRecorder::new(),
        })
    }

    pub fn run(&mut self) {
        loop {
            self.poll_events();
        }
    }

    pub fn poll_events(&mut self) -> usize {
        let mut count = 0usize;
        while let Some(event) = {
            #[cfg(feature = "perf")]
            self.perf.begin(PerfStage::TryRead);

            let ev = self.reader.try_read();

            #[cfg(feature = "perf")]
            self.perf.end(PerfStage::TryRead);

            ev
        } {
            #[cfg(feature = "perf")]
            self.perf.begin(PerfStage::OnyxTotal);

            #[cfg(feature = "perf")]
            self.perf.begin(PerfStage::ProcessEvent);

            self.process_event(&event);

            #[cfg(feature = "perf")]
            self.perf.end(PerfStage::ProcessEvent);

            #[cfg(feature = "perf")]
            self.perf.begin(PerfStage::PrefetchNext);

            self.reader.prefetch_next();

            #[cfg(feature = "perf")]
            self.perf.end(PerfStage::PrefetchNext);

            #[cfg(feature = "perf")]
            self.perf.end(PerfStage::OnyxTotal);

            core::hint::spin_loop();
            count += 1;
        }
        count
    }

    #[inline]
    fn process_event(&mut self, event: &TopOfBook) {
        self.market_state_manager.update_market_state_tob(event);
    }
}
