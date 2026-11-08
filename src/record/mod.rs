//! The recording format shared by the server and the archive step.
//!
//! A recording lives at `<root>/<YYYYMMDD>/<workspace-name>-<id>/` and a week of
//! them packs into `<root>/<YYYYMM>/<start>-<end>.tar.gz`. Both layers name
//! their paths from the values here, so a name or a date is parsed once.

pub(crate) mod archive;
pub(crate) mod read;

use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

/// A calendar date, no time and no zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Date {
    pub(crate) year: i64,
    pub(crate) month: u32,
    pub(crate) day: u32,
}

impl Date {
    pub(crate) fn from_days(days: i64) -> Self {
        let (year, month, day) = civil_from_days(days);
        Self { year, month, day }
    }

    /// Days since 1970-01-01, the inverse of `from_days`.
    pub(crate) fn days(&self) -> i64 {
        days_from_civil(self.year, self.month, self.day)
    }

    pub(crate) fn today(offset: i64) -> Self {
        Self::from_days((now_seconds() + offset).div_euclid(86_400))
    }

    pub(crate) fn compact(&self) -> String {
        format!("{:04}{:02}{:02}", self.year, self.month, self.day)
    }

    #[allow(dead_code, reason = "read by the recording viewer")]
    pub(crate) fn dashed(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    pub(crate) fn month(&self) -> String {
        format!("{:04}{:02}", self.year, self.month)
    }

    /// Day of the week, 0 for Monday.
    pub(crate) fn weekday(&self) -> u32 {
        // 1970-01-01 was a Thursday, which is 3 when Monday is 0.
        (self.days() + 3).rem_euclid(7) as u32
    }

    /// The Monday of this date's week.
    pub(crate) fn monday(&self) -> Date {
        Date::from_days(self.days() - self.weekday() as i64)
    }

    /// Read `YYYYMMDD` or `YYYY-MM-DD`.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let (year, month, day) = match text.len() {
            8 => (&text[0..4], &text[4..6], &text[6..8]),
            10 if text.as_bytes()[4] == b'-' && text.as_bytes()[7] == b'-' => {
                (&text[0..4], &text[5..7], &text[8..10])
            }
            _ => return None,
        };
        let year: i64 = year.parse().ok()?;
        let month: u32 = month.parse().ok()?;
        let day: u32 = day.parse().ok()?;
        if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
            return None;
        }
        Some(Self { year, month, day })
    }
}

/// A local date and time, taken once with an offset so the clock is not asked
/// again while a recording runs.
pub(crate) struct Local {
    pub(crate) year: i64,
    pub(crate) month: u32,
    pub(crate) day: u32,
    pub(crate) hour: u32,
    pub(crate) minute: u32,
    pub(crate) second: u32,
}

impl Local {
    pub(crate) fn now(offset: i64) -> Self {
        Self::from_seconds(now_seconds() + offset)
    }

    pub(crate) fn from_seconds(seconds: i64) -> Self {
        let days = seconds.div_euclid(86_400);
        let rest = seconds.rem_euclid(86_400);
        let (year, month, day) = civil_from_days(days);
        Self {
            year,
            month,
            day,
            hour: (rest / 3600) as u32,
            minute: ((rest % 3600) / 60) as u32,
            second: (rest % 60) as u32,
        }
    }
}

pub(crate) fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

/// The directory part of a workspace name. Separators and control characters
/// would move the recording out of its place, so they become `_`.
pub(crate) fn dir_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|ch| match ch {
            '/' | '\\' => '_',
            ch if ch.is_control() => '_',
            ch => ch,
        })
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        return "_".to_string();
    }
    cleaned.to_string()
}

pub(crate) fn workspace_dir(name: &str, id: u64) -> String {
    format!("{}-{id}", dir_name(name))
}

/// Read back `<name>-<id>`; the id has to be the last dash-separated part.
pub(crate) fn parse_workspace_dir(dir: &str) -> Option<(String, u64)> {
    let (name, id) = dir.rsplit_once('-')?;
    let id: u64 = id.parse().ok()?;
    if name.is_empty() {
        return None;
    }
    Some((name.to_string(), id))
}

/// ISO 8601 local time with the UTC offset, e.g. `2026-11-08T22:31:05+09:00`.
pub(crate) fn iso8601(offset: i64, at: SystemTime) -> String {
    let seconds = at
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0);
    let local = Local::from_seconds(seconds + offset);
    let sign = if offset < 0 { '-' } else { '+' };
    let absolute = offset.abs();
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{}{:02}:{:02}",
        local.year,
        local.month,
        local.day,
        local.hour,
        local.minute,
        local.second,
        sign,
        absolute / 3600,
        (absolute % 3600) / 60
    )
}

/// The local UTC offset, asked once of `date` so no timezone database is needed.
pub(crate) fn local_offset_seconds() -> i64 {
    let output = Command::new("date")
        .arg("+%z")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    let Ok(output) = output else {
        return 0;
    };
    if !output.status.success() {
        return 0;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    parse_offset(text.trim()).unwrap_or(0)
}

fn parse_offset(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    if bytes.len() != 5 {
        return None;
    }
    let sign = match bytes[0] {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let hours: i64 = text[1..3].parse().ok()?;
    let minutes: i64 = text[3..5].parse().ok()?;
    if hours > 14 || minutes > 59 {
        return None;
    }
    Some(sign * (hours * 3600 + minutes * 60))
}

/// Civil date of a day count since 1970-01-01.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = (shifted - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_part + 2) / 5 + 1) as u32;
    let month = if month_part < 10 {
        month_part + 3
    } else {
        month_part - 9
    } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// Day count since 1970-01-01 for a civil date.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month = month as i64;
    let day = day as i64;
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_known_day_becomes_its_date() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_675), (2026, 8, 10));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
    }

    #[test]
    fn a_date_round_trips_through_its_day_count() {
        for days in [-1, 0, 11_016, 20_675, 100_000] {
            assert_eq!(Date::from_days(days).days(), days);
        }
        assert_eq!(Date::from_days(0).compact(), "19700101");
        assert_eq!(Date::from_days(0).dashed(), "1970-01-01");
        assert_eq!(Date::from_days(0).month(), "197001");
    }

    #[test]
    fn monday_starts_the_week() {
        let thursday = Date::from_days(0);
        assert_eq!(thursday.weekday(), 3);
        assert_eq!(thursday.monday().compact(), "19691229");
        let monday = Date::from_days(20_668);
        assert_eq!(monday.weekday(), 0);
        assert_eq!(monday.monday(), monday);
    }

    #[test]
    fn both_date_spellings_parse() {
        let want = Date {
            year: 2026,
            month: 8,
            day: 10,
        };
        assert_eq!(Date::parse("20260810"), Some(want));
        assert_eq!(Date::parse("2026-08-10"), Some(want));
        assert_eq!(Date::parse("2026-8-10"), None);
        assert_eq!(Date::parse("hello"), None);
        assert_eq!(Date::parse("20261310"), None);
    }

    #[test]
    fn timestamps_carry_the_local_offset() {
        let at = UNIX_EPOCH + Duration::from_secs(1_760_000_000);
        assert_eq!(iso8601(9 * 3600, at), "2025-10-09T17:53:20+09:00");
        assert_eq!(iso8601(-5 * 3600, at), "2025-10-09T03:53:20-05:00");
        assert_eq!(iso8601(0, at), "2025-10-09T08:53:20+00:00");
    }

    #[test]
    fn an_offset_is_five_characters_or_nothing() {
        assert_eq!(parse_offset("+0900"), Some(32_400));
        assert_eq!(parse_offset("-0530"), Some(-19_800));
        assert_eq!(parse_offset("+0000"), Some(0));
        assert_eq!(parse_offset("UTC"), None);
        assert_eq!(parse_offset("+900"), None);
        assert_eq!(parse_offset("+0960"), None);
    }

    #[test]
    fn a_name_that_would_leave_the_directory_is_flattened() {
        assert_eq!(dir_name("work"), "work");
        assert_eq!(dir_name("a/b\\c"), "a_b_c");
        assert_eq!(dir_name(".."), "_");
        assert_eq!(dir_name("  "), "_");
        assert_eq!(dir_name("日本語"), "日本語");
    }

    #[test]
    fn a_workspace_directory_carries_its_id() {
        assert_eq!(workspace_dir("main", 1), "main-1");
        assert_eq!(workspace_dir("a/b", 7), "a_b-7");
        assert_eq!(parse_workspace_dir("main-1"), Some(("main".to_string(), 1)));
        assert_eq!(
            parse_workspace_dir("my-ws-12"),
            Some(("my-ws".to_string(), 12))
        );
        assert_eq!(parse_workspace_dir("main"), None);
        assert_eq!(parse_workspace_dir("main-x"), None);
    }
}
