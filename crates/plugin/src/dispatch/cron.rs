//! Minimal five-field cron matcher used by plugin job schedules.

/// Returns whether a five-field cron expression matches the minute containing
/// `timestamp` (Unix seconds, UTC).
#[must_use]
pub fn due(expression: &str, timestamp: i64) -> bool {
    let mut fields = expression.split_whitespace();
    let Some(minute) = fields.next() else {
        return false;
    };
    let Some(hour) = fields.next() else {
        return false;
    };
    let Some(day) = fields.next() else {
        return false;
    };
    let Some(month) = fields.next() else {
        return false;
    };
    let Some(weekday) = fields.next() else {
        return false;
    };
    if fields.next().is_some() {
        return false;
    }

    let days = timestamp.div_euclid(86_400);
    let seconds = timestamp.rem_euclid(86_400);
    let minute_of_day = seconds / 60;
    let (year, month_of_year, day_of_month, weekday_of_date) = civil_from_days(days);
    let current_minute = minute_of_day % 60;
    let current_hour = minute_of_day / 60;
    let _ = year;

    if !matches_field(minute, current_minute, 0, 59) {
        return false;
    }
    if !matches_field(hour, current_hour, 0, 23) {
        return false;
    }
    if !matches_field(month, month_of_year, 1, 12) {
        return false;
    }
    let day_matches = matches_field(day, day_of_month, 1, 31);
    let weekday_matches = matches_weekday(weekday, weekday_of_date);
    if day == "*" && weekday == "*" {
        true
    } else if day == "*" {
        weekday_matches
    } else if weekday == "*" {
        day_matches
    } else {
        day_matches || weekday_matches
    }
}

fn matches_weekday(field: &str, weekday: i64) -> bool {
    // Cron accepts both 0 and 7 for Sunday.
    matches_field(field, weekday, 0, 6) || (weekday == 0 && matches_field(field, 7, 0, 7))
}

fn matches_field(field: &str, value: i64, min: i64, max: i64) -> bool {
    for part in field.split(',') {
        if let Some(step) = part.strip_prefix("*/") {
            let Ok(step) = step.parse::<i64>() else {
                return false;
            };
            if step > 0 && (value - min).rem_euclid(step) == 0 {
                return true;
            }
            continue;
        }
        if let Some((start, end)) = part.split_once('-') {
            let (Ok(start), Ok(end)) = (start.parse::<i64>(), end.parse::<i64>()) else {
                return false;
            };
            if (start..=end).contains(&value) {
                return true;
            }
            continue;
        }
        if part == "*" {
            return true;
        }
        if let Ok(number) = part.parse::<i64>()
            && number == value
            && (min..=max).contains(&number)
        {
            return true;
        }
    }
    false
}

fn civil_from_days(days: i64) -> (i64, i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    if month <= 2 {
        year += 1;
    }
    let weekday = (days + 4).rem_euclid(7);
    (year, month, day, weekday)
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2026-11-10T12:05:00Z
    const BASE: i64 = 1_794_312_300;

    #[test]
    fn matches_every_five_minutes() {
        assert!(due("*/5 * * * *", BASE));
        assert!(!due("*/5 * * * *", BASE + 60));
    }

    #[test]
    fn matches_weekday_and_day_rules() {
        // 2026-11-10 is a Tuesday (2).
        assert!(due("5 12 * * 2", BASE));
        assert!(!due("5 12 * * 1", BASE));
        assert!(due("5 12 10 11 *", BASE));
    }

    #[test]
    fn rejects_malformed_expressions() {
        assert!(!due("* * *", BASE));
        assert!(!due("x * * * *", BASE));
    }
}
