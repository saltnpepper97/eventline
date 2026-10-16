use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Wall-clock milliseconds since the Unix epoch, for scope metadata.
pub fn current_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_millis() as u64
}

/// Wall-clock nanoseconds since the Unix epoch, for event timestamps.
pub fn current_nanos() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_nanos() as u64
}

/// Preserve monotonic clock precision, saturating rather than wrapping on overflow.
pub(crate) fn duration_nanos(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::duration_nanos;
    use std::time::Duration;

    #[test]
    fn nanosecond_conversion_preserves_sub_millisecond_precision() {
        assert_eq!(duration_nanos(Duration::from_nanos(987_654)), 987_654);
        assert_eq!(duration_nanos(Duration::from_nanos(1_234_567)), 1_234_567);
    }

    #[test]
    fn nanosecond_conversion_saturates_instead_of_wrapping() {
        assert_eq!(duration_nanos(Duration::ZERO), 0);
        assert_eq!(duration_nanos(Duration::from_nanos(u64::MAX)), u64::MAX);
        assert_eq!(duration_nanos(Duration::from_secs(u64::MAX)), u64::MAX);
    }
}
