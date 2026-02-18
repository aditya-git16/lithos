use lithos_events::TopOfBook;
use lithos_icc::BroadcastReader;
use onyx_core::MarketStateManager;
use std::path::Path;

pub struct OnyxEngine {
    pub market_state_manager: MarketStateManager,
    pub reader: BroadcastReader<TopOfBook>,
}

impl OnyxEngine {
    pub fn new<P: AsRef<Path>>(path: P) -> std::io::Result<Self> {
        let market_state_manager = MarketStateManager::new();
        let reader = BroadcastReader::<TopOfBook>::open(path)?;
        Ok(OnyxEngine {
            market_state_manager,
            reader,
        })
    }

    pub fn run(&mut self) {
        loop {
            self.poll_events();
        }
    }

    /// Drain all available events from the ring buffer.
    pub fn poll_events(&mut self) -> usize {
        let mut count = 0usize;
        while let Some(event) = self.reader.try_read() {
            self.process_event(&event);
            self.reader.prefetch_next();
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
