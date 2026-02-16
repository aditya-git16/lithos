use lithos_events::{SymbolId, TopOfBook};
use lithos_icc::{BroadcastWriter, RingConfig};
use obsidian_config::config::ObsidianConfig;
use obsidian_core::dto::BinanceDto;
use obsidian_util::floating_parse::{parse_px_2dp, parse_qty_3dp};
use obsidian_util::timestamp::now_ns;
use tracing::info;
use tracing_subscriber::EnvFilter;
use tungstenite::{Message, connect};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config_path = "/Users/adityaanand/dev/lithos/config/obsidian/config.toml";
    let config = ObsidianConfig::load(&config_path)?;

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("INFO")),
        )
        .init();

    let mut bus = BroadcastWriter::<TopOfBook>::create(
        &config.shm_file_path,
        RingConfig::new(config.capacity),
    )
    .expect("failed to create mmap ring");

    info!(
        "OBSIDIAN: publishing TopOfBook to {path} (cap={capacity})",
        path = &config.shm_file_path,
        capacity = config.capacity
    );

    let (mut socket, _resposne) = connect(&config.binance_ws_url).expect("failed to connect");
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

    Ok(())
}
