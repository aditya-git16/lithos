// MarketStateManager — per-symbol market state storage
//
// -------
// OnyxEngine (see onyx-engine/src/lib.rs) needs a container that maps
//     SymbolId  ->  MarketsState
// on every incoming TOB event.  This is the absolute hot path: every
// nanosecond of lookup overhead is multiplied by millions of events per
// second.  The data structure must be:
//   1. O(1) lookup with minimal constant factor
//   2. Cache-friendly (avoid pointer chasing / random memory jumps)
//   3. Zero allocation after initialisation (no heap churn on the hot path)
//   4. Simple — fewer branches, fewer instructions
//
// SymbolId is a newtype over u16 (see lithos-events).
//   • The key space is bounded: 0..65 535
//   • IDs are assigned by us, so they are dense and start near 0
//   • 65 536 is a *small* number for modern hardware
//
// This means we can use the SymbolId value directly as an array index
// eliminating hashing, comparison, and probing entirely.
//
// 1. std::collections::HashMap<SymbolId, MarketsState>
//    ─────────────────────────────────────────────────
//    • SipHash per lookup (~15-30 ns) — way too expensive on the hot path
//    • Open-addressing with robin-hood probing still involves:
//        – hash computation
//        – key comparison
//        – potential cache miss chasing buckets
//    • Heap-allocated, non-contiguous memory layout
//
// 2. rustc_hash::FxHashMap<SymbolId, MarketsState>
//    ──────────────────────────────────────────────
//    • Uses FxHash (multiply-shift) — much faster hash (~3-5 ns)
//    • Still has probing / capacity overhead / branch for key equality
//    • Better than std but still inferior to direct indexing when the key
//      is a small integer
//
// 4. Vec<Option<MarketsState>>  indexed by symbol_id.0
//    ──────────────────────────────────────────────────
//    • O(1) direct index — no hashing at all
//    • Option<T> adds size overhead (discriminant + alignment padding)
//      and introduces a branch on every access (is_some / unwrap)
//    • Verdict:    Close, but the Option wrapper is unnecessary overhead
//                   if we pre-initialise every slot.
//
// 5. Vec<MarketsState>  indexed by symbol_id.0
//    ────────────────────────────────────────────────
//    • TRUE O(1): a single array-index instruction, no hashing, no
//      comparison, no branching
//    • Contiguous heap allocation — sequential scans are perfectly
//      cache-friendly (prefetcher loves linear memory)
//    • Pre-allocate to MAX_SYMBOLS at startup; after that, zero heap
//      allocations on the hot path
//    • Memory cost: 65 536 × sizeof(MarketsState).
//      MarketsState currently contains:
//        symbol_id  (u16 + padding)  ≈  8 bytes
//        last_tob   (TopOfBook)      ≈ 48 bytes
//        last_update_ns (u64)        =  8 bytes
//        mid_x2     (i64)            =  8 bytes
//        spread_ticks (i64)          =  8 bytes
//                                     ──────────
//                              total ≈ 80 bytes  (may be ~88 with alignment)
//      65 536 × 88 = ~5.6 MB — fits comfortably in L3 cache and is
//      negligible compared to the total memory footprint of the process.
//    • To track which slots are "active" (i.e. which symbols are
//      subscribed), keep a separate small Vec<SymbolId> or bitset.
//      This keeps the hot-path struct lean (no Option discriminant).
//
// 6. Box<[MarketsState; MAX_SYMBOLS]>  (fixed-size array on the heap)
//    ─────────────────────────────────────────────────────────────────
//    • Same performance characteristics as the Vec approach
//    • Compile-time fixed size — no capacity field, slightly smaller pointer
//    • Downside: generic const requires knowing MAX_SYMBOLS at compile time
//      and Default impl for large arrays can be awkward
//    • Vec is more ergonomic in practice.
//
//
// Use a flat Vec<MarketsState> pre-allocated to MAX_SYMBOLS (65 536) entries,
// indexed directly by SymbolId.0 as usize.
//
// Lookup pattern on hot path:
//
//     let state = &mut self.states[symbol_id.0 as usize];
//
// This compiles down to a single base + offset memory access —
// the fastest possible lookup on modern hardware (no branches, no hashing).
//
// Additionally, maintain a Vec<SymbolId> (or compact bitset) of active
// symbols for iteration purposes (e.g. displaying all subscribed markets
// in a dashboard).  This secondary index is only touched on subscribe /
// unsubscribe, never on the hot TOB-update path.
//
// Summary
//
//  Data Structure            | Lookup  | Cache   | Alloc   | Complexity
//  ─────────────────────────-+─────────+─────────+─────────+───────────
//  std HashMap               | ~20 ns  | Poor    | Yes     | Medium
//  FxHashMap                 | ~5 ns   | Fair    | Yes     | Medium
//  Vec<Option<T>> indexed    | ~1 ns   | Great   | No      | Low
//  Vec<T> indexed            | <1 ns   | Great   | No      | Minimal
//  Box<[T; N]>               | <1 ns   | Great   | No      | Minimal

use lithos_events::TopOfBook;

use crate::market_state::MarketsState;

///  Max symbols we will track
pub const MAX_SYMBOLS: usize = 256;

pub struct MarketStateManager {
    markets: [MarketsState; MAX_SYMBOLS],
}

impl MarketStateManager {
    /// Builds a new manager with all slots initialised to default market state.
    ///
    /// `std::array::from_fn(f)` constructs an array `[T; N]` by calling `f(i)` for
    /// each index `i` in `0..N`. The closure receives the index and returns the
    /// value for that slot — so we get one `MarketsState` per slot without writing
    /// a literal list of 256 elements. Here we ignore the index and use the same
    /// default (zero/empty) state for every slot; they get overwritten when TOB
    /// events arrive for each symbol.
    pub fn new() -> Self {
        Self {
            markets: std::array::from_fn(|i| MarketsState::default_from_index(i)),
        }
    }

    pub fn update_market_state_tob(&mut self, tob: &TopOfBook)  {
        // Use symbol id as array index: SymbolId is a newtype over u16,
        // so .0 gives the raw value; usize is required for indexing.
        let tob_symbol = tob.symbol_id.0 as usize;

        // gets mutable ref to the market state at that index
        // using unsafe to prevent implicit check on bound and prevent branching
        let market = &mut unsafe { self.markets.get_unchecked_mut(tob_symbol) };

        market.update_state_tob(&tob);
    }
}
