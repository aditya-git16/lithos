use lithos_events::{SymbolId, TopOfBook};
use lithos_icc::BroadcastWriter;
use obsidian_config::config::ConnectionConfig;
use obsidian_core::dto::BinanceDto;
use obsidian_util::binance_book_ticker::parse_binance_book_ticker_fast;
use obsidian_util::floating_parse::{parse_px_2dp, parse_qty_3dp};
use obsidian_util::timestamp::now_ns;
use sonic_rs::from_slice;
use std::io;
use std::net::TcpStream;
use std::path::Path;
#[cfg(debug_assertions)]
use tracing::debug;
use tracing::warn;
use tungstenite::stream::{MaybeTlsStream, NoDelay};
use tungstenite::{Message, WebSocket, connect};

pub type WebsocketStream = WebSocket<MaybeTlsStream<TcpStream>>;

pub struct ObsidianEngine {
    pub socket: WebsocketStream,
    pub writer: BroadcastWriter<TopOfBook>,
    pub symbol_id: SymbolId,
}

impl ObsidianEngine {
    pub fn new<P: AsRef<Path>>(
        path: P,
        connection: &ConnectionConfig,
        symbol_id: SymbolId,
    ) -> std::io::Result<Self> {
        let (mut socket, _response) = connect(&connection.url)
            .map_err(|e| io::Error::other(format!("connect failed: {e}")))?;
        // Set TCP_NODELAY to true to disable Nagle's algorithm.
        // This instructs the OS to send small packets immediately rather than buffering them,
        // reducing latency at the cost of possibly increasing network overhead.
        if let Err(e) = socket.get_mut().set_nodelay(true) {
            warn!(?e, "failed to set TCP_NODELAY");
        }
        let writer = BroadcastWriter::<TopOfBook>::open(path)?;
        Ok(ObsidianEngine {
            socket,
            writer,
            symbol_id,
        })
    }

    pub fn run(&mut self) {
        loop {
            let data = match self.socket.read() {
                Ok(data) => data,
                Err(e) => {
                    warn!(?e, "socket read failed; stopping obsidian engine loop");
                    break;
                }
            };

            match data {
                Message::Text(text) => {
                    let text_str: &str = text.as_ref();
                    let (b, b_qty, a, a_qty) =
                        if let Some(fast) = parse_binance_book_ticker_fast(text_str) {
                            (fast.b, fast.b_qty, fast.a, fast.a_qty)
                        } else {
                            let dto: BinanceDto = match from_slice(text_str.as_bytes()) {
                                Ok(dto) => dto,
                                Err(e) => {
                                    warn!(?e, "unable to parse websocket payload");
                                    continue;
                                }
                            };
                            (dto.b, dto.b_qty, dto.a, dto.a_qty)
                        };

                    let tob = TopOfBook {
                        ts_event_ns: now_ns(),
                        symbol_id: self.symbol_id,
                        bid_px_ticks: parse_px_2dp(b),
                        bid_qty_lots: parse_qty_3dp(b_qty),
                        ask_px_ticks: parse_px_2dp(a),
                        ask_qty_lots: parse_qty_3dp(a_qty),
                    };
                    self.writer.publish(tob);
                    #[cfg(debug_assertions)]
                    {
                        let symbol_id = tob.symbol_id.0;
                        debug!("market_state[{}]: {:?}", symbol_id, tob);
                    }
                }
                Message::Ping(payload) => {
                    self.socket.write(Message::Pong(payload)).ok();
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
    }
}
