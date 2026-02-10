use onyx_config::OnyxConfig;
use onyx_engine::OnyxEngine;
use tracing_subscriber::EnvFilter;

// it would be better if the config is read from a file
// for starting i can think of path and log level ?

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config_path = "/Users/adityaanand/dev/lithos/config/onyx/config.toml";
    let config = OnyxConfig::load(&config_path)?;
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&config.log_level)),
        )
        .init();

    tracing::info!(?config, "onyx starting");

    let mut onyx_engine =
        OnyxEngine::new(config.shm_file_path).expect("failed to start onyx engine");
    onyx_engine.run();

    Ok(())
}
