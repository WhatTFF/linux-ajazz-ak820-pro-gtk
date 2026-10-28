//! Clock encoding for `SET_TEMPORARY_COMMAND_DATA` (`0x34`).

/// A local wall-clock time, free of any date library.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CivilTime {
    /// Full year (only the last two digits are sent).
    pub year: u16,
    /// 1–12.
    pub month: u8,
    /// 1–31.
    pub day: u8,
    /// 0–23.
    pub hour: u8,
    /// 0–59.
    pub minute: u8,
    /// 0–59.
    pub second: u8,
    /// 0 = Sunday … 6 = Saturday.
    pub weekday: u8,
}

impl CivilTime {
    /// Build from a date and time, deriving the weekday.
    pub const fn from_ymd_hms(
        year: u16,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: u8,
    ) -> Self {
        Self {
            year,
            month,
            day,
            hour,
            minute,
            second,
            weekday: weekday_from_ymd(year, month, day),
        }
    }
}

/// Day of week for a Gregorian date (Sakamoto's algorithm). 0 = Sunday.
pub const fn weekday_from_ymd(year: u16, month: u8, day: u8) -> u8 {
    const OFFSETS: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let mut y = year as i32;
    if month < 3 {
        y -= 1;
    }
    let mut idx = (month as usize).saturating_sub(1);
    if idx > 11 {
        idx = 11;
    }
    let w = (y + y / 4 - y / 100 + y / 400 + OFFSETS[idx] + day as i32) % 7;
    ((w + 7) % 7) as u8
}

/// Encode the 24-byte clock payload: `5A 01 5A YY MM DD hh mm ss wd`, rest zero.
pub const fn clock_payload(t: CivilTime) -> [u8; 24] {
    let mut out = [0u8; 24];
    out[0] = 0x5A;
    out[1] = 0x01;
    out[2] = 0x5A;
    out[3] = (t.year % 100) as u8;
    out[4] = t.month;
    out[5] = t.day;
    out[6] = t.hour;
    out[7] = t.minute;
    out[8] = t.second;
    out[9] = t.weekday % 7;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_layout() {
        let t = CivilTime::from_ymd_hms(2026, 10, 27, 13, 45, 9);
        let p = clock_payload(t);
        assert_eq!(&p[..10], &[0x5A, 0x01, 0x5A, 26, 10, 27, 13, 45, 9, 2]);
        assert!(p[10..].iter().all(|&b| b == 0));
    }

    #[test]
    fn weekday_math() {
        assert_eq!(weekday_from_ymd(2026, 1, 1), 4, "2026-01-01 is a Thursday");
        assert_eq!(weekday_from_ymd(2026, 10, 27), 2, "2026-10-27 is a Tuesday");
        assert_eq!(
            weekday_from_ymd(2024, 2, 29),
            4,
            "leap day 2024 is a Thursday"
        );
        assert_eq!(weekday_from_ymd(2000, 1, 1), 6, "2000-01-01 is a Saturday");
        assert_eq!(weekday_from_ymd(1970, 1, 1), 4, "epoch was a Thursday");
    }

    #[test]
    fn year_is_truncated_to_two_digits() {
        let p = clock_payload(CivilTime::from_ymd_hms(2099, 12, 31, 23, 59, 59));
        assert_eq!(p[3], 99);
        assert_eq!(&p[4..10], &[12, 31, 23, 59, 59, 4]);
    }

    #[test]
    fn invalid_month_does_not_panic() {
        // Defensive: out-of-range months must not panic in const evaluation.
        let _ = weekday_from_ymd(2026, 0, 1);
        let _ = weekday_from_ymd(2026, 13, 1);
    }
}
