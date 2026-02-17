#![forbid(unsafe_code)]

// SymbolId is consistent and stable across all processes
// repr(transparent) -> ensures that the struct memory layout is same as its single field
// Using a tuple struct (newtype pattern) that wraps u16
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct SymbolId(pub u16);

// Defining a minimal market struct ( for for initial setup and testing purposes)
// POD -> Plain old data , fixed-size
#[repr(C, packed)]
#[derive(Clone, Copy, Debug, Default)]
pub struct TopOfBook {
    pub ts_event_ns: u64,
    pub symbol_id: SymbolId,
    pub bid_px_ticks: i64, // tick -> smallest unit a price can move , tick_price -> tick_size ( 0.01 ) * actual_price (99.98) = 9998
    pub bid_qty_lots: i64, // lots -> smallest allowed quantity step , qty_lot -> lot_size (0.001) * actual_qty (0.001) = 1
    pub ask_px_ticks: i64,
    pub ask_qty_lots: i64,
}

impl TopOfBook {
    #[inline] // Function body is directly copied to call site
    pub fn mid_ticks(&self) -> i64 {
        (self.bid_px_ticks + self.ask_px_ticks) / 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{align_of, size_of};

    /// Tests that TopOfBook maintains size and alignment constraints critical for performance.
    ///
    /// Reasoning:
    /// - Size <= 64 bytes: Ensures TopOfBook fits within a single CPU cache line (64 bytes).
    ///   Keeping the struct small reduces cache misses and improves memory access patterns
    ///   when processing market data events.
    ///
    /// - Alignment <= 8 bytes: Ensures the struct can be efficiently aligned in memory without
    ///   excessive padding. This is important for:
    ///   1. Memory-mapped I/O (mmap) compatibility - many systems expect 8-byte alignment
    ///   2. Efficient atomic operations
    ///   3. Consistent memory layout across different architectures
    #[test]
    fn tob_is_small_and_aligned() {
        assert_eq!(size_of::<TopOfBook>(), 42, "TopOfBook layout changed");
        assert_eq!(align_of::<TopOfBook>(), 1, "TopOfBook should be packed");
    }

    /// Tests that SymbolId has the expected size of 2 bytes (u16).
    ///
    /// Reasoning:
    /// - SymbolId wraps a u16, so it should be exactly 2 bytes in size.
    /// - The #[repr(transparent)] attribute ensures SymbolId has the same memory layout as u16,
    ///   meaning no padding or additional fields are added.
    /// - This test verifies that SymbolId is Plain Old Data (POD), which is important for:
    ///   1. Zero-copy deserialization from binary formats
    ///   2. Safe transmutation if needed
    ///   3. Memory-mapped I/O where the exact size must be known
    ///   4. Inter-process communication where consistent size is critical
    #[test]
    fn symbol_id_is_pod() {
        assert_eq!(size_of::<SymbolId>(), 2);
    }
}
