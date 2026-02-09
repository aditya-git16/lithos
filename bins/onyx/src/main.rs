use onyx_engine::OnyxEngine;
use tracing_subscriber::EnvFilter;

fn main() {
    let path = "/tmp/lithos_md_bus";

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let mut onyx_engine = OnyxEngine::new(path).expect("failed to start onyx engine");
    onyx_engine.run();
}
