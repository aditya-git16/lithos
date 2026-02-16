use lithos_events::{Event, SymbolId, TopOfBook};
use lithos_icc::BroadcastWriter;
use obsidian_config::config::ConnectionConfig;
use obsidian_core::dto::BinanceDto;
use obsidian_util::floating_parse::{parse_px_2dp, parse_qty_3dp};
use obsidian_util::timestamp::now_ns;
use std::net::TcpStream;
use std::path::Path;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket, connect};

pub type WebsocketStream = WebSocket<MaybeTlsStream<TcpStream>>;

pub struct ObsidianEngine {
    pub socket: WebsocketStream,
    pub writer: BroadcastWriter<Event>,
    pub symbol_id: SymbolId,
}

impl ObsidianEngine {
    pub fn new<P: AsRef<Path>>(path: P, connection: &ConnectionConfig, symbol_id: SymbolId) -> std::io::Result<Self> {
        let (socket, _resposne) = connect(&connection.url).expect("failed to connect");
        let writer = BroadcastWriter::<Event>::open(path)?;
        Ok(ObsidianEngine {
            socket: socket,
            writer: writer,
            symbol_id: symbol_id,
        })
    }

    pub fn run(&mut self) {
        loop {
            let data = self.socket.read().expect("unable to read data");

            match data {
                Message::Text(text) => {
                    let dto: BinanceDto = unsafe {
                        sonic_rs::from_slice_unchecked(text.as_ref()).expect("unable to parse")
                    };

                    let tob = TopOfBook {
                        ts_event_ns: now_ns(),
                        symbol_id: self.symbol_id,
                        bid_px_ticks: parse_px_2dp(&dto.b),
                        bid_qty_lots: parse_qty_3dp(&dto.b_qty),
                        ask_px_ticks: parse_px_2dp(&dto.a),
                        ask_qty_lots: parse_qty_3dp(&dto.a_qty),
                    };
                    self.writer.publish(Event::TopOfBook(tob));
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
