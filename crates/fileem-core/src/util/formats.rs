//! Human readable sizes, counts and dates.

use chrono::{DateTime, Local};
use std::time::SystemTime;

const UNITS: [&str; 6] = ["KB", "MB", "GB", "TB", "PB", "EB"];

/// Size with binary units and three significant digits; empty for `None` (unknown / folder).
pub fn size(bytes: Option<u64>) -> String {
    let Some(bytes) = bytes else {
        return String::new();
    };
    if bytes < 1024 {
        return format!("{bytes} {}", if bytes == 1 { "byte" } else { "bytes" });
    }
    let mut v = bytes as f64;
    let mut unit = 0;
    v /= 1024.0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    let num = if v >= 100.0 {
        format!("{v:.0}")
    } else if v >= 10.0 {
        format!("{v:.1}")
    } else {
        format!("{v:.2}")
    };
    format!("{num} {}", UNITS[unit])
}

/// "1,234,567 bytes".
pub fn exact(bytes: u64) -> String {
    format!("{} bytes", count(bytes))
}

/// "1.18 MB (1,234,567 bytes)", or just the size below 1 KB.
pub fn size_with_exact(bytes: u64) -> String {
    if bytes < 1024 {
        size(Some(bytes))
    } else {
        format!("{} ({})", size(Some(bytes)), exact(bytes))
    }
}

/// A number with thousands separators.
pub fn count(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// "Jan 5, 2024, 3:04 PM" in local time; empty for `None`.
pub fn date(t: Option<SystemTime>) -> String {
    t.map(|t| DateTime::<Local>::from(t).format("%b %-d, %Y, %-I:%M %p").to_string())
        .unwrap_or_default()
}

/// "January 5, 2024 at 3:04:05 PM" in local time; empty for `None`.
pub fn date_long(t: Option<SystemTime>) -> String {
    t.map(|t| DateTime::<Local>::from(t).format("%B %-d, %Y at %-I:%M:%S %p").to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_use_binary_units_with_three_significant_digits() {
        assert_eq!(size(None), "");
        assert_eq!(size(Some(0)), "0 bytes");
        assert_eq!(size(Some(1)), "1 byte");
        assert_eq!(size(Some(1023)), "1023 bytes");
        assert_eq!(size(Some(1024)), "1.00 KB");
        assert_eq!(size(Some(1536)), "1.50 KB");
        assert_eq!(size(Some(10 * 1024)), "10.0 KB");
        assert_eq!(size(Some(100 * 1024)), "100 KB");
        assert_eq!(size(Some(1 << 20)), "1.00 MB");
        assert_eq!(size(Some(1 << 30)), "1.00 GB");
        assert_eq!(size(Some(i64::MAX as u64)), "8.00 EB");
        assert_eq!(size(Some(u64::MAX)), "16.0 EB");
    }

    #[test]
    fn exact_and_count_use_grouping() {
        assert_eq!(exact(1_234_567), "1,234,567 bytes");
        assert_eq!(count(1000), "1,000");
        assert_eq!(count(999), "999");
        assert_eq!(count(0), "0");
        assert_eq!(size_with_exact(1_234_567), "1.18 MB (1,234,567 bytes)");
        assert_eq!(size_with_exact(12), "12 bytes");
    }

    #[test]
    fn dates_handle_none() {
        assert_eq!(date(None), "");
        assert!(!date(Some(SystemTime::UNIX_EPOCH)).is_empty());
        assert!(!date_long(Some(SystemTime::UNIX_EPOCH)).is_empty());
    }
}
