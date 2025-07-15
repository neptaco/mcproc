/// Timestamp formatting utilities
use chrono::{DateTime, Local, Utc};

/// Format a prost timestamp to local time string
pub fn format_timestamp_local(timestamp: Option<&prost_types::Timestamp>) -> String {
    timestamp
        .and_then(|ts| {
            DateTime::<Utc>::from_timestamp(ts.seconds, ts.nanos as u32).map(|utc| {
                utc.with_timezone(&Local)
                    .format("%Y-%m-%d %H:%M:%S")
                    .to_string()
            })
        })
        .unwrap_or_default()
}

/// Format UTC datetime to RFC 3339 format with milliseconds
pub fn format_datetime_utc_with_tz(dt: DateTime<Utc>) -> String {
    // Use RFC 3339 format for better compatibility
    // Example: 2025-07-15T03:13:12.375+00:00
    dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
