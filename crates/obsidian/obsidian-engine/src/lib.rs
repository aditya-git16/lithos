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

#[cfg(feature = "perf")]
use lithos_perf_recorder::{PerfRecorder, PerfStage};

pub type WebsocketStream = WebSocket<MaybeTlsStream<TcpStream>>;

/// Socket-free hot path. Usable standalone for perf testing.
pub struct ObsidianProcessor {
    pub writer: BroadcastWriter<TopOfBook>,
    pub symbol_id: SymbolId,
    #[cfg(feature = "perf")]
    pub perf: PerfRecorder,
}

impl ObsidianProcessor {
    pub fn new<P: AsRef<Path>>(path: P, symbol_id: SymbolId) -> io::Result<Self> {
        let writer = BroadcastWriter::<TopOfBook>::open(path)?;
        Ok(Self {
            writer,
            symbol_id,
            #[cfg(feature = "perf")]
            perf: PerfRecorder::new(),
        })
    }

    /// The instrumented hot path — identical logic to production.
    /// Returns `true` if a TOB was published, `false` on parse failure.
    #[inline]
    pub fn process_text(&mut self, text: &str) -> bool {
        #[cfg(feature = "perf")]
        self.perf.begin(PerfStage::ObsidianTotal);

        // ── JSON parsing ──
        #[cfg(feature = "perf")]
        self.perf.begin(PerfStage::ParseJson);

        let parsed = if let Some(fast) = parse_binance_book_ticker_fast(text) {
            Some((fast.b, fast.b_qty, fast.a, fast.a_qty))
        } else {
            let dto: Result<BinanceDto, _> = from_slice(text.as_bytes());
            match dto {
                Ok(dto) => Some((dto.b, dto.b_qty, dto.a, dto.a_qty)),
                Err(e) => {
                    warn!(?e, "unable to parse websocket payload");
                    #[cfg(feature = "perf")]
                    self.perf.end(PerfStage::ParseJson);
                    #[cfg(feature = "perf")]
                    self.perf.end(PerfStage::ObsidianTotal);
                    return false;
                }
            }
        };

        #[cfg(feature = "perf")]
        self.perf.end(PerfStage::ParseJson);

        let (b, b_qty, a, a_qty) = parsed.unwrap();

        // ── Numeric conversion ──
        #[cfg(feature = "perf")]
        self.perf.begin(PerfStage::ParseNumeric);

        let bid_px = parse_px_2dp(b);
        let bid_qty = parse_qty_3dp(b_qty);
        let ask_px = parse_px_2dp(a);
        let ask_qty = parse_qty_3dp(a_qty);

        #[cfg(feature = "perf")]
        self.perf.end(PerfStage::ParseNumeric);

        // ── Timestamp ──
        #[cfg(feature = "perf")]
        self.perf.begin(PerfStage::TimestampEvent);

        let ts = now_ns();

        #[cfg(feature = "perf")]
        self.perf.end(PerfStage::TimestampEvent);

        // ── Build TOB ──
        #[cfg(feature = "perf")]
        self.perf.begin(PerfStage::BuildTob);

        let tob = TopOfBook {
            ts_event_ns: ts,
            symbol_id: self.symbol_id,
            bid_px_ticks: bid_px,
            bid_qty_lots: bid_qty,
            ask_px_ticks: ask_px,
            ask_qty_lots: ask_qty,
        };

        #[cfg(feature = "perf")]
        self.perf.end(PerfStage::BuildTob);

        // ── Publish ──
        #[cfg(feature = "perf")]
        self.perf.begin(PerfStage::Publish);

        self.writer.publish(tob);

        #[cfg(feature = "perf")]
        self.perf.end(PerfStage::Publish);

        #[cfg(debug_assertions)]
        {
            let symbol_id = tob.symbol_id.0;
            debug!("market_state[{}]: {:?}", symbol_id, tob);
        }

        #[cfg(feature = "perf")]
        self.perf.end(PerfStage::ObsidianTotal);

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
