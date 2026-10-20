//! Formatting helpers shared by the drivers: dates, fractional seconds, binary previews.

const MAX_BINARY_PREVIEW: usize = 32;

/// `0x` + hex of the first 32 bytes, with a length suffix when truncated.
pub fn format_bytes(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return "0x".into();
    }
    let preview = &bytes[..bytes.len().min(MAX_BINARY_PREVIEW)];
    let hex: String = preview.iter().map(|b| format!("{b:02X}")).collect();
    if bytes.len() > MAX_BINARY_PREVIEW {
        format!("0x{hex}\u{2026} ({} bytes)", bytes.len())
    } else {
        format!("0x{hex}")
    }
}

/// `.1234567`-style fraction with trailing zeros trimmed; empty when zero.
pub fn fraction(nanos: u32) -> String {
    if nanos == 0 {
        return String::new();
    }
    // Seven digits (100 ns ticks), like the .NET formatting the old app used.
    let ticks = nanos / 100;
    let s = format!("{ticks:07}");
    format!(".{}", s.trim_end_matches('0'))
}

pub fn format_naive_datetime(dt: &chrono::NaiveDateTime) -> String {
    use chrono::Timelike;
    format!("{}{}", dt.format("%Y-%m-%d %H:%M:%S"), fraction(dt.nanosecond()))
}

pub fn format_naive_time(t: &chrono::NaiveTime) -> String {
    use chrono::Timelike;
    format!("{}{}", t.format("%H:%M:%S"), fraction(t.nanosecond()))
}

pub fn format_offset(offset_minutes: i32) -> String {
    let sign = if offset_minutes < 0 { '-' } else { '+' };
    let m = offset_minutes.abs();
    format!(" {sign}{:02}:{:02}", m / 60, m % 60)
}

/// f64 without an exponent for "normal" magnitudes, shortest round-trip otherwise.
pub fn format_f64(v: f64) -> String {
    if v.is_finite() && v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_preview() {
        assert_eq!(format_bytes(&[]), "0x");
        assert_eq!(format_bytes(&[1, 255]), "0x01FF");
        let long: Vec<u8> = (0..40).collect();
        let s = format_bytes(&long);
        assert!(s.starts_with("0x000102"));
        assert!(s.ends_with("\u{2026} (40 bytes)"));
    }

    #[test]
    fn datetime_fraction_trims_zeros() {
        let dt = chrono::NaiveDate::from_ymd_opt(2024, 3, 5).unwrap().and_hms_milli_opt(7, 8, 9, 120).unwrap();
        assert_eq!(format_naive_datetime(&dt), "2024-03-05 07:08:09.12");
        let dt = chrono::NaiveDate::from_ymd_opt(2024, 3, 5).unwrap().and_hms_opt(7, 8, 9).unwrap();
        assert_eq!(format_naive_datetime(&dt), "2024-03-05 07:08:09");
    }

    #[test]
    fn offsets() {
        assert_eq!(format_offset(-330), " -05:30");
        assert_eq!(format_offset(0), " +00:00");
    }

    #[test]
    fn floats() {
        assert_eq!(format_f64(1.5), "1.5");
        assert_eq!(format_f64(3.0), "3");
        assert_eq!(format_f64(0.1 + 0.2), "0.30000000000000004");
    }
}
