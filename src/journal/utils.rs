use std::time::{SystemTime, UNIX_EPOCH};

/// Get current system time in milliseconds since UNIX epoch
pub fn current_millis() -> u64 {
    let now = SystemTime::now();
    now.duration_since(UNIX_EPOCH)
        .expect("SystemTime before UNIX_EPOCH")
        .as_millis() as u64
}

/// Convert milliseconds since UNIX epoch to a human-readable local timestamp
pub fn millis_to_local(ms: u64) -> String {
    use chrono::{Local, TimeZone};
    let dt = Local.timestamp_millis_opt(ms as i64).single()
        .unwrap_or_else(|| Local.timestamp_millis_opt(0).single().unwrap());
    dt.format("%Y-%m-%d %H:%M:%S").to_string()
}
