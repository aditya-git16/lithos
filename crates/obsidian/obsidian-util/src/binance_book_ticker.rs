/// Borrowed view over the Binance bookTicker fields needed on the hot path.
#[derive(Debug, Clone, Copy)]
pub struct BinanceBookTickerView<'a> {
    pub b: &'a str,
    pub b_qty: &'a str,
    pub a: &'a str,
    pub a_qty: &'a str,
}

/// Fast, allocation-free parser for Binance `bookTicker` JSON payloads.
///
/// Overview
/// - This is a minimal, hot-path parser optimized for speed and zero allocations.
/// - It scans the UTF-8 byte buffer for simple key/value fragments of the form:
///     "<k>":"<value>"
///   where `<k>` is a single ASCII character key and `<value>` is a quoted string.
/// - The parser only cares about the four keys used by Binance `bookTicker`:
///     - `b` : best bid price (string)
///     - `B` : best bid quantity (string)
///     - `a` : best ask price (string)
///     - `A` : best ask quantity (string)
///
/// Algorithm details
/// 1. Work on the raw byte slice of the input (`input.as_bytes()`), using unchecked
///    indexing for performance. This means we perform manual bounds checks where needed.
/// 2. Iterate with index `i` and look for a pattern that begins with a double-quote
///    at `i`, followed by a single-character key at `i+1`, then `"` `:` `"` at
///    `i+2`, `i+3`, `i+4` respectively. The loop condition `i + 5 <= len` ensures
///    there are enough bytes remaining for that minimal pattern.
/// 3. Once the `"<k>":"` prefix is recognized, set `value_start = i + 5` and scan
///    forward with `j` until the next double-quote which terminates the quoted value.
///    If no terminating quote is found before the end of the buffer, return `None`.
/// 4. Convert the slice `value_start..j` to `&str` with `from_utf8_unchecked`. This
///    avoids validation allocations under the assumption that Binance payloads are valid UTF-8.
/// 5. Match the single-byte `key` against `b`, `B`, `a`, `A`. For each field found,
///    store the borrowed `&str` slice and set the corresponding bit in `found` to
///    avoid duplicate assignments.
/// 6. When all four bits are set (tracked via `ALL_FIELDS`), return a
///    `BinanceBookTickerView` containing the four borrowed string slices.
/// 7. Otherwise, advance `i` to `j + 1` (past the closing quote) and continue scanning.
///
/// Safety / limitations
/// - This parser is intentionally conservative: it only recognizes the very simple
///   `"<k>":"<value>"` fragments and does not attempt to parse numbers, nested
///   structures, escaped quotes inside values, or other JSON edge-cases.
/// - It uses `unsafe` unchecked indexing and `from_utf8_unchecked` for speed. Callers
///   must treat a `None` result as an indication to fall back to a general JSON parser.
/// - Keys must be single ASCII characters and values must be simple quoted strings
///   without embedded escaped quotes (the common case for Binance `bookTicker` payloads).
///
#[inline(always)]
pub fn parse_binance_book_ticker_fast(input: &str) -> Option<BinanceBookTickerView<'_>> {
    let b = input.as_bytes();
    let len = b.len();
    let mut i = 0usize;

    let mut bid_px = "";
    let mut bid_qty = "";
    let mut ask_px = "";
    let mut ask_qty = "";

    let mut found = 0u8;
    const BID_PX: u8 = 1 << 0;
    const BID_QTY: u8 = 1 << 1;
    const ASK_PX: u8 = 1 << 2;
    const ASK_QTY: u8 = 1 << 3;
    const ALL_FIELDS: u8 = BID_PX | BID_QTY | ASK_PX | ASK_QTY;

    while i + 5 <= len {
        // Detect key/value fragment: "<k>":"<value>"
        if unsafe { *b.get_unchecked(i) } != b'"' {
            i += 1;
            continue;
        }
        let key = unsafe { *b.get_unchecked(i + 1) };
        if unsafe {
            *b.get_unchecked(i + 2) != b'"'
                || *b.get_unchecked(i + 3) != b':'
                || *b.get_unchecked(i + 4) != b'"'
        } {
            i += 1;
            continue;
        }

        let value_start = i + 5;
        let mut j = value_start;
        while j < len {
            if unsafe { *b.get_unchecked(j) } == b'"' {
                break;
            }
            j += 1;
        }
        if j >= len {
            return None;
        }

        let value = unsafe { std::str::from_utf8_unchecked(b.get_unchecked(value_start..j)) };

        match key {
            b'b' if (found & BID_PX) == 0 => {
                bid_px = value;
                found |= BID_PX;
            }
            b'B' if (found & BID_QTY) == 0 => {
                bid_qty = value;
                found |= BID_QTY;
            }
            b'a' if (found & ASK_PX) == 0 => {
                ask_px = value;
                found |= ASK_PX;
            }
            b'A' if (found & ASK_QTY) == 0 => {
                ask_qty = value;
                found |= ASK_QTY;
            }
            _ => {}
        }

        if found == ALL_FIELDS {
            return Some(BinanceBookTickerView {
                b: bid_px,
                b_qty: bid_qty,
                a: ask_px,
                a_qty: ask_qty,
            });
        }

        i = j + 1;
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_standard_book_ticker_payload() {
        let msg =
            r#"{"u":123,"s":"BTCUSDT","b":"12345.67","B":"0.123","a":"12345.68","A":"0.456"}"#;
        let parsed = parse_binance_book_ticker_fast(msg).expect("parser should succeed");
        assert_eq!(parsed.b, "12345.67");
        assert_eq!(parsed.b_qty, "0.123");
        assert_eq!(parsed.a, "12345.68");
        assert_eq!(parsed.a_qty, "0.456");
    }

    #[test]
    fn parses_even_when_field_order_changes() {
        let msg =
            r#"{"s":"BTCUSDT","a":"12345.68","A":"0.456","u":123,"b":"12345.67","B":"0.123"}"#;
        let parsed = parse_binance_book_ticker_fast(msg).expect("parser should succeed");
        assert_eq!(parsed.b, "12345.67");
        assert_eq!(parsed.b_qty, "0.123");
        assert_eq!(parsed.a, "12345.68");
        assert_eq!(parsed.a_qty, "0.456");
    }

    #[test]
    fn returns_none_for_missing_fields() {
        let msg = r#"{"u":123,"s":"BTCUSDT","b":"1.0","a":"2.0"}"#;
        assert!(parse_binance_book_ticker_fast(msg).is_none());
    }
}
