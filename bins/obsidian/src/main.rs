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
    pub b: String,    // best bid price
    #[serde(rename = "B")]
    pub b_qty: String,    // best bid qty
    pub a: String,    // best ask price
    #[serde(rename = "A")]
    pub a_qty: String,    // best ask qty
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
                let bid_px: f64 = dto.b.parse().expect("invalid bid price");
                let bid_qty: f64 = dto.b_qty.parse().expect("invalid bid qty");
                let ask_px: f64 = dto.a.parse().expect("invalid ask price");
                let ask_qty: f64 = dto.a_qty.parse().expect("invalid ask qty");
                let tick_size = 0.01;
                let lot_size = 0.001;
                let tob = TopOfBook {
                    ts_event_ns : now_ns(),
                    symbol_id : SymbolId(1),
                    bid_px_ticks: (bid_px / tick_size).round() as i64,
                    bid_qty_lots: (bid_qty / lot_size).round() as i64,
                    ask_px_ticks: (ask_px / tick_size).round() as i64,
                    ask_qty_lots: (ask_qty / lot_size).round() as i64,
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
