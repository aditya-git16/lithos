use lithos_events::Event;
use lithos_icc::BroadcastWriter;
use obsidian_core::websocket_manager::WebsocketManager;

pub struct ObsidianEngine{
    pub ws_manager: WebsocketManager,
    pub writer: BroadcastWriter<Event>
}