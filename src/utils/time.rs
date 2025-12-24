use std::time::{SystemTime, UNIX_EPOCH, Instant};

pub fn now_ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub fn now_instant() -> Instant {
    Instant::now()
}

pub fn elapsed_ms(start: Instant) -> u64 {
    start.elapsed().as_millis() as u64
}

pub fn elapsed_us(start: Instant) -> u64 {
    start.elapsed().as_micros() as u64
}
