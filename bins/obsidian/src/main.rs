use lithos_events::{SymbolId, TopOfBook};
use lithos_icc::{BroadcastWriter, RingConfig};
use serde::Deserialize;
use tungstenite::{Message, connect};

fn now_ns() -> u64 {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    t.as_nanos() as u64
}

#[derive(Debug, Deserialize)]
pub struct BinanceDto {
    pub u: u64,    // order book updateId
    pub s: String, // symbol
    pub b: String, // best bid price
    #[serde(rename = "B")]
    pub b_qty: String, // best bid qty
    pub a: String, // best ask price
    #[serde(rename = "A")]
    pub a_qty: String, // best ask qty
}

fn main() {
    let path = "/tmp/lithos_md_bus";
    let capacity = 1 << 16;

    let mut bus = BroadcastWriter::<TopOfBook>::create(path, RingConfig::new(capacity))
        .expect("failed to create mmap ring");

    eprintln!("OBSIDIAN: publishing TopOfBook to {path} (cap={capacity})");

    let (mut socket, _resposne) =
        connect("wss://stream.binance.com:9443/ws/btcusdt@bookTicker").expect("failed to connect");
    println!("Connected to Binance Websocket Server");

    loop {
        let data = socket.read().expect("unable to read data");

        match data {
            Message::Text(text) => {
                let dto: BinanceDto = serde_json::from_str(&text).expect("unable to parse");

                let tob = TopOfBook {
                    ts_event_ns: now_ns(),
                    symbol_id: SymbolId(1),
                    bid_px_ticks: parse_px_2dp(&dto.b),
                    bid_qty_lots: parse_qty_3dp(&dto.b_qty),
                    ask_px_ticks: parse_px_2dp(&dto.a),
                    ask_qty_lots: parse_qty_3dp(&dto.a_qty),
                };
                bus.publish(tob);
            }
            Message::Ping(payload) => {
                socket.write(Message::Pong(payload)).ok();
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
}

/// Parses a price string with 2 decimal places into a fixed-point integer.
/// Example: "123.45" -> 12345 (representing 123.45 as integer ticks)
#[inline(always)]
fn parse_px_2dp(s: &str) -> i64 {
    parse_fixed_dp::<2>(s)
}

/// Parses a quantity string with 3 decimal places into a fixed-point integer.
/// Example: "12.345" -> 12345 (representing 12.345 as integer lots)
#[inline(always)]
fn parse_qty_3dp(s: &str) -> i64 {
    parse_fixed_dp::<3>(s)
}

/// Computes 10^DP at compile time for efficient fixed-point arithmetic.
/// This avoids runtime exponentiation and allows the compiler to optimize.
/// Examples: pow10::<2>() = 100, pow10::<3>() = 1000
#[inline(always)]
fn pow10<const DP: u32>() -> i64 {
    // compile-time constant evaluation - compiler optimizes this match away
    match DP {
        0 => 1,              // 10^0 = 1
        1 => 10,             // 10^1 = 10
        2 => 100,            // 10^2 = 100
        3 => 1000,           // 10^3 = 1000
        4 => 10_000,         // 10^4 = 10,000
        5 => 100_000,        // 10^5 = 100,000
        6 => 1_000_000,      // 10^6 = 1,000,000
        _ => 10_i64.pow(DP), // fallback (won’t be hit for DP=2/3)
    }
}

/// Parses a decimal string into a fixed-point integer representation.
///
/// Algorithm overview:
/// 1. Handle optional negative sign
/// 2. Parse integer part (digits before decimal point)
/// 3. Parse fractional part (digits after decimal point, up to DP digits)
/// 4. Pad fractional part with zeros if fewer than DP digits were provided
/// 5. Combine: sign * (int_part * 10^DP + frac_part)
///
/// Example: parse_fixed_dp::<2>("-123.45")
///   - sign = -1
///   - int_part = 123
///   - frac = 45
///   - result = -1 * (123 * 100 + 45) = -12345
///
/// Example: parse_fixed_dp::<3>("12.3")
///   - sign = 1
///   - int_part = 12
///   - frac = 3, then padded to 300
///   - result = 1 * (12 * 1000 + 300) = 12300
#[inline(always)]
fn parse_fixed_dp<const DP: u32>(s: &str) -> i64 {
    // Convert string to byte slice for efficient character-by-character parsing
    let b = s.as_bytes();
    // Current position/index in the byte slice
    let mut i = 0usize;

    // 1: Handle optional negative sign
    // Initialize sign to positive (1)
    let mut sign = 1i64;
    // Check if first character is a minus sign
    if i < b.len() && b[i] == b'-' {
        sign = -1;  // Set sign to negative
        i += 1;      // Advance past the minus sign
    }

    // 2: Parse integer part (digits before the decimal point)
    // Accumulate integer digits into int_part
    let mut int_part = 0i64;
    // Loop through characters until we hit the decimal point or end of string
    while i < b.len() {
        let c = b[i];  // Get current byte/character
        // If we encounter a decimal point, we've finished the integer part
        if c == b'.' {
            i += 1;    // Advance past the decimal point
            break;     // Exit the integer parsing loop
        }
        // Convert ASCII digit to integer value and accumulate
        // '0' = 48 in ASCII, so (c - b'0') converts '0'..'9' to 0..9
        // Multiply existing value by 10 and add new digit (standard base-10 parsing)
        int_part = int_part * 10 + (c - b'0') as i64;
        i += 1;        // Move to next character
    }

    // 3: Parse fractional part (digits after the decimal point)
    // Accumulate fractional digits into frac
    let mut frac = 0i64;
    // Track how many fractional digits we've actually parsed
    let mut got = 0u32;
    // Loop through remaining characters, but only parse up to DP digits
    while i < b.len() && got < DP {
        let c = b[i];  // Get current byte/character
        // If we hit a non-digit character, stop parsing (e.g., trailing whitespace)
        if c < b'0' || c > b'9' {
            break;     // Exit fractional parsing loop
        }
        // Convert ASCII digit to integer and accumulate (same as integer part)
        frac = frac * 10 + (c - b'0') as i64;
        got += 1;      // Increment count of parsed fractional digits
        i += 1;        // Move to next character
    }

    // 4: Pad fractional part with zeros if we didn't get enough digits
    // This ensures we always have exactly DP fractional digits
    // Example: "12.3" with DP=3 becomes "12.300" -> frac = 300
    while got < DP {
        frac *= 10;    // Multiply by 10 to shift left (equivalent to adding a zero)
        got += 1;      // Increment count to track padding
    }

    // 5: Combine integer and fractional parts into final fixed-point integer
    // Formula: sign * (int_part * 10^DP + frac)
    // - int_part * pow10::<DP>() shifts integer part left by DP places
    // - Adding frac gives us the complete fixed-point representation
    // - Multiplying by sign handles negative numbers
    // Example: "123.45" with DP=2 -> sign=1, int_part=123, frac=45
    //          Result = 1 * (123 * 100 + 45) = 12345
    sign * (int_part * pow10::<DP>() + frac)
}
