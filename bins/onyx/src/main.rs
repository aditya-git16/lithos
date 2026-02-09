use onyx_engine::OnyxEngine;

fn main() {
    let path = "/tmp/lithos_md_bus";
    let mut onyx_engine = OnyxEngine::new(path).expect("failed to start onyx engine");
    onyx_engine.run();
}
