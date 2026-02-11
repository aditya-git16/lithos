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
                    ts_event_ns: now_ns(),  // consider dto.E if available
                    symbol_id: SymbolId(1), // map dto.s -> SymbolId once in a table
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

#[inline(always)]
fn parse_px_2dp(s: &str) -> i64 {
    parse_fixed_dp::<2>(s)
}
#[inline(always)]
fn parse_qty_3dp(s: &str) -> i64 {
    parse_fixed_dp::<3>(s)
}

#[inline(always)]
fn pow10<const DP: u32>() -> i64 {
    // compile-time
    match DP {
        0 => 1,
        1 => 10,
        2 => 100,
        3 => 1000,
        4 => 10_000,
        5 => 100_000,
        6 => 1_000_000,
        _ => 10_i64.pow(DP), // fallback (won’t be hit for DP=2/3)
    }
}

#[inline(always)]
fn parse_fixed_dp<const DP: u32>(s: &str) -> i64 {
    let b = s.as_bytes();
    let mut i = 0usize;

    let mut sign = 1i64;
    if i < b.len() && b[i] == b'-' {
        sign = -1;
        i += 1;
    }

    let mut int_part = 0i64;
    while i < b.len() {
        let c = b[i];
        if c == b'.' {
            i += 1;
            break;
        }
        int_part = int_part * 10 + (c - b'0') as i64;
        i += 1;
    }

    let mut frac = 0i64;
    let mut got = 0u32;
    while i < b.len() && got < DP {
        let c = b[i];
        if c < b'0' || c > b'9' {
            break;
        }
        frac = frac * 10 + (c - b'0') as i64;
        got += 1;
        i += 1;
    }

    while got < DP {
        frac *= 10;
        got += 1;
    }

    sign * (int_part * pow10::<DP>() + frac)
}
