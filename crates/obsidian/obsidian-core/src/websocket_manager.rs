pub const MAX_SYMBOLS: usize = 256;

#[derive(Default)]
pub struct WebsocketSymbolState {
    pub book_tikcer: String,
}

pub struct WebsocketManager {
    pub websocket_connections: [WebsocketSymbolState; MAX_SYMBOLS],
}

impl WebsocketManager {
    pub fn new() -> Self {
        Self {
            websocket_connections: std::array::from_fn(|_| WebsocketSymbolState::default()),
        }
    }
}
