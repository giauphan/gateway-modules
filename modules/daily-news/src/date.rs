//! Pure date/time parsing, formatting, and boundary calculation.

fn days_from_civil(y: i64, m: u64, d: u64) -> i64 {
    let y = y - if m <= 2 { 1 } else { 0 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe as i64 - 719468
}

fn parse_date_part(date_part: &str) -> Result<i64, String> {
    let d_parts: Vec<&str> = date_part.split('-').collect();
    if d_parts.len() != 3 {
        return Err(format!("Invalid date part: {date_part}"));
    }
    let year: i64 = d_parts[0]
        .parse()
        .map_err(|e| format!("Invalid year: {e}"))?;
    let month: u64 = d_parts[1]
        .parse()
        .map_err(|e| format!("Invalid month: {e}"))?;
    let day: u64 = d_parts[2]
        .parse()
        .map_err(|e| format!("Invalid day: {e}"))?;
    if !(1..=12).contains(&month) {
        return Err(format!("Month out of range (1-12): {month}"));
    }
    let is_leap = (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
    let max_days = match month {
        2 => {
            if is_leap {
                29
            } else {
                28
            }
        }
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if day < 1 || day > max_days {
        return Err(format!(
            "Day {day} out of range for month {month} in year {year} (max {max_days})"
        ));
    }

    let days = days_from_civil(year, month, day);
    if days < 0 {
        return Err("Pre-1970 dates unsupported".to_string());
    }
    Ok(days)
}

fn parse_timezone_offset(tz_info: Option<&str>) -> Result<i64, String> {
    let tz = match tz_info {
        Some(t) => t,
        None => return Ok(0),
    };
    let sign = if tz.starts_with('-') { -1i64 } else { 1i64 };
    let raw_tz = tz.trim_start_matches(['+', '-']);
    let (off_h, off_m) = if let Some((h_str, m_str)) = raw_tz.split_once(':') {
        let h: i64 = h_str
            .parse()
            .map_err(|e| format!("Invalid timezone hour: {e}"))?;
        let m: i64 = m_str
            .parse()
            .map_err(|e| format!("Invalid timezone minute: {e}"))?;
        (h, m)
    } else if raw_tz.len() >= 4 {
        let h: i64 = raw_tz
            .get(..2)
            .ok_or_else(|| format!("Invalid timezone offset: {raw_tz}"))?
            .parse()
            .map_err(|e| format!("Invalid timezone hour: {e}"))?;
        let m: i64 = raw_tz
            .get(2..4)
            .ok_or_else(|| format!("Invalid timezone offset: {raw_tz}"))?
            .parse()
            .map_err(|e| format!("Invalid timezone minute: {e}"))?;
        (h, m)
    } else {
        let h: i64 = raw_tz
            .parse()
            .map_err(|e| format!("Invalid timezone hour: {e}"))?;
        (h, 0)
    };
    if !(0..=23).contains(&off_h) {
        return Err(format!("Timezone hour offset {off_h} out of range (0-23)"));
    }
    if !(0..=59).contains(&off_m) {
        return Err(format!("Timezone minute offset {off_m} out of range (0-59)"));
    }
    Ok(sign * (off_h * 3600 + off_m * 60))
}

fn parse_time_of_day(time_sub: &str) -> Result<i64, String> {
    let t_parts: Vec<&str> = time_sub.trim().split(':').collect();
    if t_parts.len() < 2 {
        return Err(format!("Invalid time part: {time_sub}"));
    }
    let h: i64 = t_parts[0]
        .parse()
        .map_err(|e| format!("Invalid hour: {e}"))?;
    let m: i64 = t_parts[1]
        .parse()
        .map_err(|e| format!("Invalid minute: {e}"))?;
    if !(0..=23).contains(&h) {
        return Err(format!("Hour {h} out of range (0-23)"));
    }
    if !(0..=59).contains(&m) {
        return Err(format!("Minute {m} out of range (0-59)"));
    }
    let s_val: i64 = if t_parts.len() >= 3 {
        let s_str = t_parts[2].split('.').next().unwrap_or("0");
        let s: i64 = s_str
            .parse()
            .map_err(|e| format!("Invalid second: {e}"))?;
        if !(0..=60).contains(&s) {
            return Err(format!("Second {s} out of range (0-60)"));
        }
        s
    } else {
        0
    };
    Ok(h * 3600 + m * 60 + s_val)
}

pub fn parse_iso_to_epoch(ts: &str) -> Result<u64, String> {
    let s = ts.trim();
    if s.is_empty() {
        return Err("Empty timestamp".to_string());
    }

    if let Ok(num) = s.parse::<u64>() {
        let secs = if num > 100_000_000_000 {
            num / 1000
        } else {
            num
        };
        return Ok(secs);
    }

    let (date_part, time_part) = if let Some((d, t)) = s.split_once('T') {
        (d, Some(t))
    } else if let Some((d, t)) = s.split_once(' ') {
        (d, Some(t))
    } else {
        (s, None)
    };

    let days = parse_date_part(date_part)?;
    let mut local_seconds: i64 = days * 86400;
    let mut offset_seconds: i64 = 0;

    if let Some(t_str) = time_part {
        let t_clean = t_str.trim();
        let (time_sub, tz_info) = if let Some(idx) = t_clean.find(['+', '-']) {
            (&t_clean[..idx], Some(&t_clean[idx..]))
        } else {
            (t_clean.trim_end_matches(['Z', 'z']), None)
        };

        offset_seconds = parse_timezone_offset(tz_info)?;
        local_seconds += parse_time_of_day(time_sub)?;
    }

    let utc_seconds = local_seconds - offset_seconds;
    if utc_seconds < 0 {
        return Err("Pre-1970 timestamp".to_string());
    }
    Ok(utc_seconds as u64)
}

pub fn parse_boundary_datetime(val: &str, is_end_of_day: bool) -> Result<u64, String> {
    let s = val.trim();
    if s.is_empty() {
        return Err("Empty date boundary".to_string());
    }
    if s.len() == 10 && s.chars().filter(|&c| c == '-').count() == 2 {
        let base_epoch = parse_iso_to_epoch(s)?;
        return if is_end_of_day {
            Ok(base_epoch + 86399)
        } else {
            Ok(base_epoch)
        };
    }
    parse_iso_to_epoch(s)
}

pub fn current_utc_timestamp() -> String {
    let epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(1793618000);
    format_utc_timestamp(epoch)
}

pub fn format_utc_timestamp(epoch_secs: u64) -> String {
    let days = epoch_secs / 86400;
    let rem_secs = epoch_secs % 86400;
    let hours = rem_secs / 3600;
    let minutes = (rem_secs % 3600) / 60;
    let seconds = rem_secs % 60;

    let mut year = 1970;
    let mut d = days;
    loop {
        let leap = (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
        let days_in_year = if leap { 366 } else { 365 };
        if d < days_in_year {
            break;
        }
        d -= days_in_year;
        year += 1;
    }
    let leap = (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
    let days_in_months = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 1;
    for &dim in &days_in_months {
        if d < dim {
            break;
        }
        d -= dim;
        month += 1;
    }
    let day = d + 1;
    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}+00:00")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_date_boundary_and_epoch() {
        let start = parse_boundary_datetime("2026-10-28", false).unwrap();
        let end = parse_boundary_datetime("2026-10-28", true).unwrap();
        assert_eq!(end - start, 86399);

        let iso = parse_boundary_datetime("2026-11-01T15:30:00Z", false).unwrap();
        assert!(iso > start);

        let formatted = format_utc_timestamp(iso);
        assert!(formatted.starts_with("2026-11-01"));
        assert!(formatted.ends_with("+00:00"));

        assert!(parse_boundary_datetime("not-a-valid-date", false).is_err());
        assert!(parse_iso_to_epoch("2026-02-31").is_err());
        assert!(parse_iso_to_epoch("2026-11-05T25:99:00Z").is_err());
        assert!(parse_iso_to_epoch("2026-11-05T12:00:00+99:99").is_err());
        assert!(parse_iso_to_epoch("2026-04-31").is_err());
        assert!(parse_iso_to_epoch("2024-02-29").is_ok());
        assert!(parse_iso_to_epoch("2023-02-29").is_err());
    }
}
