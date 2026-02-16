use lithos_events::{SymbolId, TopOfBook};
use lithos_icc::{BroadcastWriter, RingConfig};
use obsidian_core::dto::BinanceDto;
use obsidian_util::floating_parse::{parse_px_2dp, parse_qty_3dp};
use obsidian_util::timestamp::now_ns;
use tracing::info;
use tracing_subscriber::EnvFilter;
use tungstenite::{Message, connect};

fn main() {
    let path = "/tmp/lithos_md_bus";
    let capacity = 1 << 16;

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("INFO")),
        )
        .init();

    let mut bus = BroadcastWriter::<TopOfBook>::create(path, RingConfig::new(capacity))
        .expect("failed to create mmap ring");

    info!("OBSIDIAN: publishing TopOfBook to {path} (cap={capacity})");

    let (mut socket, _resposne) =
        connect("wss://stream.binance.com:9443/ws/btcusdt@bookTicker").expect("failed to connect");
    info!("Connected to Binance Websocket Server");

    loop {
        let data = socket.read().expect("unable to read data");

        match data {
            Message::Text(text) => {
                let dto: BinanceDto = unsafe {
                    sonic_rs::from_slice_unchecked(text.as_ref()).expect("unable to parse")
                };

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
