//! Canonical JSON encoding and hashing for AERF payloads.
//! Used by: signature, chain, and Merkle verification.

use std::cmp::Ordering;

use serde_json::{Map, Number, Value};
use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

use crate::aerf::{AerfError, AerfResult};

const MAX_SAFE_INTEGER: i128 = 9_007_199_254_740_991;

#[derive(Clone, Copy)]
enum NumberMode {
    Strict,
    Verbatim,
}

pub fn canonicalize(value: &Value) -> AerfResult<String> {
    encode(value, "$", NumberMode::Strict)
}

pub(crate) fn canonicalize_verbatim(value: &Value) -> AerfResult<String> {
    encode(value, "$", NumberMode::Verbatim)
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn encode(value: &Value, path: &str, mode: NumberMode) -> AerfResult<String> {
    match value {
        Value::Null => Ok("null".to_owned()),
        Value::Bool(true) => Ok("true".to_owned()),
        Value::Bool(false) => Ok("false".to_owned()),
        Value::String(string) => Ok(encode_string(string)),
        Value::Number(number) => encode_number(number, path, mode),
        Value::Array(values) => encode_array(values, path, mode),
        Value::Object(object) => encode_object(object, path, mode),
    }
}

fn encode_array(values: &[Value], path: &str, mode: NumberMode) -> AerfResult<String> {
    let mut out = String::from("[");
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(&encode(value, &format!("{path}[{index}]"), mode)?);
    }
    out.push(']');
    Ok(out)
}

fn encode_object(object: &Map<String, Value>, path: &str, mode: NumberMode) -> AerfResult<String> {
    let mut keys = Vec::with_capacity(object.len());
    for raw in object.keys() {
        let normalized = raw.nfc().collect::<String>();
        keys.push((raw.as_str(), normalized));
    }

    keys.sort_by(|left, right| compare_code_points(&left.1, &right.1));

    for pair in keys.windows(2) {
        if pair[0].1 == pair[1].1 {
            return Err(AerfError::DuplicateNormalizedKey {
                path: path.to_owned(),
                key: pair[1].0.to_owned(),
            });
        }
    }

    let mut out = String::from("{");
    for (index, (raw, normalized)) in keys.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(&encode_string(normalized));
        out.push(':');
        let child_path = join_key(path, normalized);
        let value = object
            .get(*raw)
            .ok_or_else(|| AerfError::UnsupportedValue {
                path: child_path.clone(),
                kind: "missing object value".to_owned(),
            })?;
        out.push_str(&encode(value, &child_path, mode)?);
    }
    out.push('}');
    Ok(out)
}

fn encode_string(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.nfc() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000C}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ if u32::from(ch) <= 0x1f => {
                out.push_str(&format!("\\u{:04x}", u32::from(ch)));
            }
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

fn encode_number(number: &Number, path: &str, mode: NumberMode) -> AerfResult<String> {
    let lexeme = number.to_string();
    if matches!(mode, NumberMode::Verbatim) {
        return Ok(lexeme);
    }

    if let Some(integer) = parse_plain_integer(&lexeme, path)? {
        return Ok(integer.to_string());
    }

    let value = lexeme
        .parse::<f64>()
        .map_err(|_| AerfError::CanonicalNumber {
            path: path.to_owned(),
            message: format!("Invalid number ({lexeme})"),
        })?;

    if !value.is_finite() {
        return Err(AerfError::CanonicalNumber {
            path: path.to_owned(),
            message: format!("Non-finite number ({lexeme})"),
        });
    }

    if value == 0.0 && lexeme.starts_with('-') {
        return Ok("0".to_owned());
    }

    if value.fract() != 0.0 {
        return Err(AerfError::CanonicalNumber {
            path: path.to_owned(),
            message: format!(
                "Non-integer finite number ({lexeme}): canonical JSON cannot round-trip fractional numbers"
            ),
        });
    }

    if value.abs() > MAX_SAFE_INTEGER as f64 {
        return Err(AerfError::CanonicalNumber {
            path: path.to_owned(),
            message: format!("Unsafe integer ({lexeme}): exceeds Number.MAX_SAFE_INTEGER"),
        });
    }

    Ok(format!("{value:.0}"))
}

fn parse_plain_integer(lexeme: &str, path: &str) -> AerfResult<Option<i128>> {
    if lexeme.contains('.') || lexeme.contains('e') || lexeme.contains('E') {
        return Ok(None);
    }

    let integer = lexeme
        .parse::<i128>()
        .map_err(|_| AerfError::CanonicalNumber {
            path: path.to_owned(),
            message: format!("Invalid integer ({lexeme})"),
        })?;

    if integer.abs() > MAX_SAFE_INTEGER {
        return Err(AerfError::CanonicalNumber {
            path: path.to_owned(),
            message: format!("Unsafe integer ({lexeme}): exceeds Number.MAX_SAFE_INTEGER"),
        });
    }

    Ok(Some(integer))
}

fn compare_code_points(left: &str, right: &str) -> Ordering {
    let mut left_chars = left.chars();
    let mut right_chars = right.chars();

    loop {
        match (left_chars.next(), right_chars.next()) {
            (Some(a), Some(b)) => {
                let ordering = u32::from(a).cmp(&u32::from(b));
                if ordering != Ordering::Equal {
                    return ordering;
                }
            }
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (None, None) => return Ordering::Equal,
        }
    }
}

fn join_key(path: &str, key: &str) -> String {
    if path == "$" {
        format!("$.{key}")
    } else {
        format!("{path}.{key}")
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{canonicalize, canonicalize_verbatim};

    #[test]
    fn sorts_keys_and_normalizes_strings() {
        let value = json!({
            "e\u{301}": "Ame\u{301}lie",
            "a": 1
        });

        let canonical = canonicalize(&value).expect("canonical");

        assert_eq!(canonical, "{\"a\":1,\"é\":\"Amélie\"}");
    }

    #[test]
    fn replays_verbatim_number_lexemes() {
        let value: serde_json::Value = serde_json::from_str("{\"amount\":1250.0}").expect("parse");

        let canonical = canonicalize_verbatim(&value).expect("canonical");

        assert_eq!(canonical, "{\"amount\":1250.0}");
    }
}
