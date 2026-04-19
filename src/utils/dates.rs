use anyhow::{Context, Result};

/// Parse an optional `--as-of` argument into a Unix timestamp.
pub fn parse_as_of(s: Option<&str>) -> Result<Option<i64>> {
    match s {
        None => Ok(None),
        Some(v) => parse_iso8601_to_epoch(v)
            .with_context(|| {
                format!(
                    "parsing --as-of '{v}': expected ISO 8601 (e.g. 2026-03-15 or 2026-03-15T10:00:00)"
                )
            })
            .map(Some),
    }
}

/// Parse an ISO 8601 date or datetime string to a unix epoch (seconds, UTC).
fn parse_iso8601_to_epoch(s: &str) -> Result<i64> {
    let s = s.trim_end_matches('Z');

    let (date_part, time_part) =
        if let Some((d, t)) = s.split_once('T').or_else(|| s.split_once(' ')) {
            (d, Some(t))
        } else {
            (s, None)
        };

    let parse_u32 = |v: &str, label: &str| -> Result<u32> {
        v.parse::<u32>()
            .with_context(|| format!("invalid {label} in '{s}'"))
    };

    let date_parts: Vec<&str> = date_part.split('-').collect();
    if date_parts.len() != 3 {
        anyhow::bail!("expected YYYY-MM-DD in '{s}'");
    }
    let year = parse_u32(date_parts[0], "year")?;
    let month = parse_u32(date_parts[1], "month")?;
    let day = parse_u32(date_parts[2], "day")?;

    let (hour, minute, second) = if let Some(t) = time_part {
        let parts: Vec<&str> = t.split(':').collect();
        if parts.len() < 2 {
            anyhow::bail!("expected HH:MM or HH:MM:SS in '{s}'");
        }
        let h = parse_u32(parts[0], "hour")?;
        let m = parse_u32(parts[1], "minute")?;
        let sec = if parts.len() >= 3 {
            parse_u32(parts[2], "second")?
        } else {
            0
        };
        (h, m, sec)
    } else {
        (0, 0, 0)
    };

    if month == 0 || month > 12 {
        anyhow::bail!("month out of range in '{s}'");
    }
    if day == 0 || day > 31 {
        anyhow::bail!("day out of range in '{s}'");
    }
    if hour > 23 || minute > 59 || second > 59 {
        anyhow::bail!("time out of range in '{s}'");
    }

    let y = year as i64;
    let m = month as i64;
    let d = day as i64;
    let days = days_since_epoch(y, m, d)?;
    let epoch = days * 86400 + (hour as i64) * 3600 + (minute as i64) * 60 + (second as i64);
    Ok(epoch)
}

fn days_since_epoch(year: i64, month: i64, day: i64) -> Result<i64> {
    let y = if month <= 2 { year - 1 } else { year };
    let m = if month <= 2 { month + 9 } else { month - 3 };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * m + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let rd = era * 146097 + doe;
    Ok(rd - 719_468)
}
