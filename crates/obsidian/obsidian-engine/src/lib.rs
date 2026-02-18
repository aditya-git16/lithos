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

/// Socket-free hot path. Usable standalone for perf testing.
pub struct ObsidianProcessor {
    pub writer: BroadcastWriter<TopOfBook>,
    pub symbol_id: SymbolId,
}

impl ObsidianProcessor {
    pub fn new<P: AsRef<Path>>(path: P, symbol_id: SymbolId) -> io::Result<Self> {
        let writer = BroadcastWriter::<TopOfBook>::open(path)?;
        Ok(Self { writer, symbol_id })
    }

    /// The hot path — identical logic to production.
    /// Returns `true` if a TOB was published, `false` on parse failure.
    #[inline]
    pub fn process_text(&mut self, text: &str) -> bool {
        // ── JSON parsing ──
        let parsed = if let Some(fast) = parse_binance_book_ticker_fast(text) {
            Some((fast.b, fast.b_qty, fast.a, fast.a_qty))
        } else {
            let dto: Result<BinanceDto, _> = from_slice(text.as_bytes());
            match dto {
                Ok(dto) => Some((dto.b, dto.b_qty, dto.a, dto.a_qty)),
                Err(e) => {
                    warn!(?e, "unable to parse websocket payload");
                    return false;
                }
            }
        };

        let (b, b_qty, a, a_qty) = parsed.unwrap();

        // ── Numeric conversion ──
        let bid_px = parse_px_2dp(b);
        let bid_qty = parse_qty_3dp(b_qty);
        let ask_px = parse_px_2dp(a);
        let ask_qty = parse_qty_3dp(a_qty);

        // ── Timestamp + Build + Publish ──
        let ts = now_ns();
        let tob = TopOfBook {
            ts_event_ns: ts,
            symbol_id: self.symbol_id,
            bid_px_ticks: bid_px,
            bid_qty_lots: bid_qty,
            ask_px_ticks: ask_px,
            ask_qty_lots: ask_qty,
        };
        self.writer.publish(tob);

        #[cfg(debug_assertions)]
        {
            let symbol_id = tob.symbol_id.0;
            debug!("market_state[{}]: {:?}", symbol_id, tob);
        }

        true
    }
}

/// Full engine: processor + WebSocket I/O.
pub struct ObsidianEngine {
    pub processor: ObsidianProcessor,
    pub socket: WebsocketStream,
}

impl ObsidianEngine {
    pub fn new<P: AsRef<Path>>(
        path: P,
        connection: &ConnectionConfig,
        symbol_id: SymbolId,
    ) -> io::Result<Self> {
        let (mut socket, _response) = connect(&connection.url)
            .map_err(|e| io::Error::other(format!("connect failed: {e}")))?;
        if let Err(e) = socket.get_mut().set_nodelay(true) {
            warn!(?e, "failed to set TCP_NODELAY");
        }
        let processor = ObsidianProcessor::new(path, symbol_id)?;
        Ok(ObsidianEngine { processor, socket })
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
                    self.processor.process_text(text.as_ref());
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
