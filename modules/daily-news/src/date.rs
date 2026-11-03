//! Pure date/time parsing, formatting, and boundary calculation.

fn days_from_civil(y: i64, m: u64, d: u64) -> i64 {
    let y = y - if m <= 2 { 1 } else { 0 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe as i64 - 719468
}

pub fn parse_iso_to_epoch(ts: &str) -> Result<u64, String> {
    let s = ts.trim();
    if s.is_empty() {
        return Err("Empty timestamp".to_string());
    }

    let (date_part, time_part) = if let Some((d, t)) = s.split_once('T') {
        (d, Some(t))
    } else if let Some((d, t)) = s.split_once(' ') {
        (d, Some(t))
    } else {
        (s, None)
    };

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
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return Err(format!("Date out of range: {date_part}"));
    }

    let days = days_from_civil(year, month, day);
    if days < 0 {
        return Err("Pre-1970 dates unsupported".to_string());
    }

    let mut seconds = (days as u64) * 86400;
    if let Some(t_str) = time_part {
        let t_clean = t_str.trim_end_matches('Z');
        let t_clean = t_clean.split('+').next().unwrap_or(t_clean);
        let t_parts: Vec<&str> = t_clean.split(':').collect();
        if t_parts.len() >= 2 {
            let h: u64 = t_parts[0].parse().unwrap_or(0);
            let m: u64 = t_parts[1].parse().unwrap_or(0);
            let s_val: u64 = if t_parts.len() >= 3 {
                t_parts[2]
                    .split('.')
                    .next()
                    .unwrap_or("0")
                    .parse()
                    .unwrap_or(0)
            } else {
                0
            };
            seconds += h * 3600 + m * 60 + s_val;
        }
    }
    Ok(seconds)
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
    }
}
