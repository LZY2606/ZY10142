use serde_json::Value;
use std::cmp::Ordering;

pub fn parse_time_ns(value: &Value) -> Result<i64, String> {
    match value {
        Value::Number(number) => number
            .as_i64()
            .ok_or_else(|| "integer timestamps must be nanoseconds".to_string()),
        Value::String(text) => parse_rfc3339_ns(text),
        _ => Err("timestamp must be an integer or RFC 3339 string".to_string()),
    }
}

pub fn parse_rfc3339_ns(input: &str) -> Result<i64, String> {
    let bytes = input.as_bytes();
    let zulu = input.ends_with('Z');
    let offset_pos = if zulu {
        bytes.len() - 1
    } else {
        input
            .rfind(['+', '-'])
            .filter(|index| *index > 10)
            .ok_or("timestamp requires a timezone")?
    };
    let date_time = &input[..offset_pos];
    let (date, clock) = date_time
        .split_once('T')
        .ok_or("timestamp must use T separator")?;
    let (year, rest) = date.split_once('-').ok_or("invalid timestamp date")?;
    let (month, day) = rest.split_once('-').ok_or("invalid timestamp date")?;
    let (hour, rest) = clock.split_once(':').ok_or("invalid timestamp time")?;
    let (minute, second_part) = rest.split_once(':').ok_or("invalid timestamp time")?;
    let (second, fraction) = match second_part.split_once('.') {
        Some((second, fraction)) => (second, Some(fraction)),
        None => (second_part, None),
    };
    let mut nanos: i64 = 0;
    if let Some(fraction) = fraction {
        if fraction.is_empty()
            || fraction.len() > 9
            || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err("timestamp fractional seconds must contain 1-9 digits".into());
        }
        nanos = fraction
            .parse::<i64>()
            .map_err(|_| "invalid timestamp fraction")?
            * 10i64.pow((9 - fraction.len()) as u32);
    }
    let mut offset_seconds = 0;
    if !zulu {
        let sign = if input.as_bytes()[offset_pos] == b'-' {
            -1
        } else {
            1
        };
        let offset = &input[offset_pos + 1..];
        let (offset_hour, offset_minute) =
            offset.split_once(':').ok_or("invalid timestamp offset")?;
        offset_seconds = sign
            * (offset_hour
                .parse::<i64>()
                .map_err(|_| "invalid timestamp offset")?
                * 3600
                + offset_minute
                    .parse::<i64>()
                    .map_err(|_| "invalid timestamp offset")?
                    * 60);
    }
    let year = year.parse::<i32>().map_err(|_| "invalid timestamp year")?;
    let month = month
        .parse::<u32>()
        .map_err(|_| "invalid timestamp month")?;
    let day = day.parse::<u32>().map_err(|_| "invalid timestamp day")?;
    let days_since_civil = days_from_civil(year as i64, month as i64, day as i64);
    let seconds = days_since_civil * 86400
        + hour.parse::<i64>().map_err(|_| "invalid timestamp hour")? * 3600
        + minute
            .parse::<i64>()
            .map_err(|_| "invalid timestamp minute")?
            * 60
        + second
            .parse::<i64>()
            .map_err(|_| "invalid timestamp second")?
        - offset_seconds;
    seconds
        .checked_mul(1_000_000_000)
        .and_then(|value| value.checked_add(nanos))
        .ok_or_else(|| "timestamp is outside supported nanosecond range".to_string())
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_adj = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * month_adj + 2) / 5 + day - 1;
    let days_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146097 + days_of_era - 719468
}

pub fn canonical_json(value: &Value) -> Vec<u8> {
    let mut output = Vec::new();
    write_canonical(value, &mut output);
    output
}

fn write_canonical(value: &Value, output: &mut Vec<u8>) {
    match value {
        Value::Null => output.extend_from_slice(b"null"),
        Value::Bool(boolean) => output.extend_from_slice(if *boolean { b"true" } else { b"false" }),
        Value::Number(number) => output.extend_from_slice(number.to_string().as_bytes()),
        Value::String(text) => {
            output.extend_from_slice(serde_json::to_string(text).unwrap().as_bytes())
        }
        Value::Array(items) => {
            output.push(b'[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                write_canonical(item, output);
            }
            output.push(b']');
        }
        Value::Object(map) => {
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            entries.sort_by(|left, right| match left.0.len().cmp(&right.0.len()) {
                Ordering::Equal => left.0.cmp(right.0),
                ordering => ordering,
            });
            output.push(b'{');
            for (index, (key, item)) in entries.iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                write_canonical(&Value::String((*key).clone()), output);
                output.push(b':');
                write_canonical(item, output);
            }
            output.push(b'}');
        }
    }
}

pub fn fingerprint128(bytes: &[u8]) -> String {
    const OFFSET: u128 = 0x6c62272e07bb014262b821756295c58d;
    const PRIME: u128 = 0x00000100000001b3;
    let mut hash = OFFSET;
    for byte in bytes {
        hash ^= *byte as u128;
        hash = hash.wrapping_mul(PRIME);
    }
    format!("{hash:032x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_timezone_and_nanoseconds() {
        assert_eq!(
            parse_rfc3339_ns("1970-01-01T09:00:00.500+09:00").unwrap(),
            500_000_000
        );
        assert_eq!(
            parse_rfc3339_ns("1970-01-01T00:00:00.000000001Z").unwrap(),
            1
        );
    }

    #[test]
    fn canonicalizes_object_keys() {
        let value = serde_json::json!({"b": 1, "a": [2, {"d": 3, "c": 4}]});
        assert_eq!(
            String::from_utf8(canonical_json(&value)).unwrap(),
            r#"{"a":[2,{"c":4,"d":3}],"b":1}"#
        );
    }
}
