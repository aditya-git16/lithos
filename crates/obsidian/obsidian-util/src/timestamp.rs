#[inline(always)]
pub fn now_ns() -> u64 {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    t.as_nanos() as u64
}
