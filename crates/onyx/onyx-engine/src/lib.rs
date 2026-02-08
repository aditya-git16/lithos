// Setup the main onyx engine
// It has the following functions
// 1. Read values from shared memory
// 2. Read values form current state ?
// 3. Perform calculations using both both values
// 4. Update market state
//
// We also have multiple markets so we need to need to define a data struct which holds
// the symbol id and market state map , (e.g something like a hashmap)

use lithos_icc::BroadcastReader;
use onyx_core::MarketStateManager;

pub struct OnyxEngine<T: Copy> {
    // per symbol market state
    // this is the state that will get updated
    pub market_state_manager: MarketStateManager,

    // A reader from shared memory ?
    // To read events , we have already defined a broadcast reader so we can use that
    // Generic T can be abstracted - > see later
    pub reader: BroadcastReader<T>,
}
