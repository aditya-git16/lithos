use lithos_events::Event;
use lithos_icc::{BroadcastWriter, RingConfig};
use obsidian_config::config::ObsidianConfig;
use obsidian_core::websocket_manager::WebsocketManager;
use std::net::TcpStream;
use std::path::Path;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{WebSocket, connect};

pub type WebsocketStream = WebSocket<MaybeTlsStream<TcpStream>>;

pub struct ObsidianEngine {
    pub ws_stream: WebsocketStream,
    pub ws_manager: WebsocketManager,
    pub writer: BroadcastWriter<Event>,
}

impl ObsidianEngine {
    pub fn new<P: AsRef<Path>>(
        path: P,
        capacity: RingConfig,
        config: &ObsidianConfig,
    ) -> std::io::Result<Self> {
        let (socket, _resposne) = connect(&config.binance_ws_url).expect("failed to connect");
        // error handling for this function also needed
        let ws_manager = WebsocketManager::new();
        let writer = BroadcastWriter::<Event>::create(path, capacity)?;
        Ok(ObsidianEngine {
            ws_stream: socket,
            ws_manager: ws_manager,
            writer: writer,
        })
    }
}
