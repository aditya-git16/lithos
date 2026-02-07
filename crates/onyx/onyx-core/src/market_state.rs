// We want to define a market state (microsturcture state) that can be used to display useful values
// about the market. The state is updated initally using TOB events. The core onyx engine will update this
// state using the TOB event values.

// How to define such a market state ?
// Thinking of the stats to display -> mid_price and stats are obvious choices so let's include in the market struct
// Also of what market is the data of ? SymbolId defined in lithos core shoud be used.
// The TOB event used to update these market stats can also be included in the market state struct

// Strust values need to be public since we will use functions to update the state

use lithos_events::{SymbolId, TopOfBook};

#[derive(Debug)]
pub struct MarketsState {
    /// Symbol of this state
    pub symbol_id: SymbolId,

    /// Top of the book for TOB for debugging purposes ?
    pub last_tob: TopOfBook,

    // We also need the timestamp of state update
    /// Timestamp of the state update (nanoseconds)
    pub last_update_ns: u64,

    // For mid price , to preserve precision and avoid global f64 usage
    // we will state mid_price * 2 instead and places where mid price is needed we
    // cast this as f64 and divide by 2 to derive actual midprice
    // We basically store the times two price to avoid precision loss
    /// Mid_price * 2 , preserves 0.5 tick precision
    /// Divide the value by 2 to get the actual midprice
    pub mid_x2: i64, // Using i64 since the prices in TOB are i64

    // Next vlaue we use is spread
    // Spread represents the cost of liquidity in the market and is a importan t basic market indicator
    /// Spread (in ticks) , ask_price - bid_price
    /// Value is always > 0 for a valid orderbook since ask_price is greater than bid_price
    pub spread_ticks: i64,
}