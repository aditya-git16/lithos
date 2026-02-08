pub mod top_of_the_book;
pub use top_of_the_book::{SymbolId, TopOfBook};

// the “type” of the event (which variant) is encoded
// in the stored bytes (discriminant + payload)

/// Wire event type for the broadcast bus. One variant is sent per message;
/// the reader matches on the discriminant to handle each kind. Must be
/// `Copy` (and `Clone`) so it can be read/written in the mmap ring buffer.
#[derive(Clone, Copy, Debug)]
pub enum Event {
    TopOfBook(TopOfBook),
}
