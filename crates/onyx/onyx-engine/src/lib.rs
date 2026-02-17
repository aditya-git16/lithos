use lithos_events::TopOfBook;
use lithos_icc::BroadcastReader;
use onyx_core::MarketStateManager;
use std::path::Path;

pub struct OnyxEngine {
    // per symbol market state
    // this is the state that will get updated
    pub market_state_manager: MarketStateManager,

    // A reader from shared memory ?
    // To read events , we have already defined a broadcast reader so we can use that
    // we replace the generic with TopOfBook as this is the type we publish.
    pub reader: BroadcastReader<TopOfBook>,
}

// Implement the functionality of the engine

impl OnyxEngine {
    // First create/initialise the engine
    pub fn new<P: AsRef<Path>>(path: P) -> std::io::Result<Self> {
        let market_state_manager = MarketStateManager::new();
        // this part can be abstracted ?
        let reader = BroadcastReader::<TopOfBook>::open(path)?;
        Ok(OnyxEngine {
            market_state_manager,
            reader,
        })
    }

    // Now we define the run function of the engine
    // it will return nothing ?
    // it runs the engine -> polls the shm , update the state
    // it runs in a constant loop and polls the shm
    pub fn run(&mut self) {
        loop {
            // poll the events , this fucntion should return , one event at a time
            self.poll_events();
        }
    }

    fn poll_events(&mut self) {
        // we use while let instead of if let because in if let we process just one event
        // but in case of while let we keep processing as long as we get events
        while let Some(event) = self.reader.try_read() {
            // then we process the event (process as in using that event to calculate state , using state + event)
            self.process_event(&event);
            self.reader.prefetch_next();
            core::hint::spin_loop()
        }
    }

    #[inline]
    fn process_event(&mut self, event: &TopOfBook) {
        self.market_state_manager.update_market_state_tob(event);
    }
}
