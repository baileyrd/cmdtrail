//! Human-friendly duration parsing for `cmdtrail prune --older-than`.
//! Accepts `<number><unit>` where unit is one of `h`/`d`/`w`/`m`/`y`
//! (hours, days, weeks, ~30-day months, ~365-day years). Deliberately not
//! calendar-aware — a retention window doesn't need calendar precision,
//! and the approximation keeps this a pure function with no timezone or
//! leap-year edge cases.

/// Parse a duration string into a number of seconds.
pub fn parse(input: &str) -> Result<i64, String> {
    let input = input.trim();
    let mut chars = input.chars();
    let unit = chars
        .next_back()
        .ok_or_else(|| "duration must not be empty".to_string())?;
    // `chars` has had its last char consumed; what remains is the numeric
    // prefix, taken via the underlying str so we never split on a byte
    // that isn't a char boundary (chars() iterates by scalar value, so
    // this is safe for any input, not just ASCII).
    let num_part = chars.as_str();

    let n: i64 = num_part.parse().map_err(|_| {
        format!("invalid duration {input:?}: expected a number followed by h/d/w/m/y, e.g. \"90d\"")
    })?;
    if n < 0 {
        return Err(format!("invalid duration {input:?}: must not be negative"));
    }

    let secs_per_unit: i64 = match unit {
        'h' => 3_600,
        'd' => 86_400,
        'w' => 7 * 86_400,
        'm' => 30 * 86_400,
        'y' => 365 * 86_400,
        other => {
            return Err(format!(
                "invalid duration {input:?}: unknown unit {other:?}, expected one of h/d/w/m/y"
            ))
        }
    };

    n.checked_mul(secs_per_unit)
        .ok_or_else(|| format!("duration {input:?} overflows"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_each_unit() {
        assert_eq!(parse("1h"), Ok(3_600));
        assert_eq!(parse("2d"), Ok(2 * 86_400));
        assert_eq!(parse("1w"), Ok(7 * 86_400));
        assert_eq!(parse("6m"), Ok(6 * 30 * 86_400));
        assert_eq!(parse("1y"), Ok(365 * 86_400));
    }

    #[test]
    fn trims_surrounding_whitespace() {
        assert_eq!(parse("  90d  "), Ok(90 * 86_400));
    }

    #[test]
    fn rejects_missing_unit() {
        assert!(parse("90").is_err());
    }

    #[test]
    fn rejects_unknown_unit() {
        assert!(parse("90x").is_err());
    }

    #[test]
    fn rejects_empty() {
        assert!(parse("").is_err());
    }

    #[test]
    fn rejects_negative() {
        assert!(parse("-5d").is_err());
    }

    #[test]
    fn rejects_non_numeric() {
        assert!(parse("abcd").is_err());
    }

    #[test]
    fn never_panics_on_multibyte_input() {
        // A non-ASCII trailing char must produce an Err, not panic on a
        // str::split_at char-boundary violation.
        assert!(parse("5°").is_err());
        assert!(parse("°").is_err());
    }
}
