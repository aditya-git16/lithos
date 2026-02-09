// Setup the main onyx engine
// It has the following functions
// 1. Read values from shared memory
// 2. Read values form current state ?
// 3. Perform calculations using both both values
// 4. Update market state
//
// We also have multiple markets so we need to need to define a data struct which holds
// the symbol id and market state map , (e.g something like a hashmap)

use std::path::Path;
use lithos_events::Event;
use lithos_icc::BroadcastReader;
use onyx_core::MarketStateManager;

pub struct OnyxEngine {
    // per symbol market state
    // this is the state that will get updated
    pub market_state_manager: MarketStateManager,

    // A reader from shared memory ?
    // To read events , we have already defined a broadcast reader so we can use that
    // we replace the generic with Event enum , this is the type we want to read
    // or more specifically a variant of this type
    pub reader: BroadcastReader<Event>,
}

// Implement the functionality of the engine

impl OnyxEngine {
    // First create/initialise the engine
    pub fn new <P : AsRef<Path>> (path : P) -> std::io::Result<Self> {
        let market_state_manager = MarketStateManager::new();
        // this part can be abstracted ?
        let reader = BroadcastReader::<Event>::open(path)?;
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
        }
    }

    #[inline]
    fn process_event(&mut self, event: &Event) {
        // we use the event here to perform calculations and update state
        // in the start the event will be tob but we will it as the generic T
        // so we match the event the event with its type and then process accordingly
        match event {
            Event::TopOfBook(tob) => {
                if let Err(e) = self.market_state_manager.update_market_state_tob(tob) {
                    eprintln!("TOB update failed : {}" , e);
                }
            }
        }
    }
}
