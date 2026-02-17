use lithos_events::{SymbolId, TopOfBook};
use lithos_icc::{BroadcastWriter, RingConfig};
use obsidian_config::config::ObsidianConfig;
use obsidian_engine::ObsidianEngine;
use tracing::info;
use tracing_subscriber::EnvFilter;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // tungstenite+rustls in this workspace is built without rustls default crypto features.
    // Install a process-level provider once before any TLS handshakes occur.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let config_path = "/Users/adityaanand/dev/lithos/config/obsidian/config.toml";
    let config = ObsidianConfig::load(&config_path)?;

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&config.log_level)),
        )
        .init();

    BroadcastWriter::<TopOfBook>::create(&config.shm_file_path, RingConfig::new(config.capacity))
        .expect("failed to create mmap ring");

    info!(
        "OBSIDIAN: publishing TopOfBook to {path} (cap={capacity})",
        path = &config.shm_file_path,
        capacity = config.capacity
    );

    for connection in config.connections {
        let path = config.shm_file_path.clone();
        std::thread::spawn(move || {
            let mut engine =
                ObsidianEngine::new(&path, &connection, SymbolId(connection.symbol_id))
                    .expect("Unable to initialise ObsidianEngine");
            engine.run();
        });
    }

    // block main thread from exiting
    loop {
        std::thread::park();
    }
}
