//! `json.dumps(v, ensure_ascii=False)` as Python prints it (`, ` and `: ` separators), so text
//! built from tool inputs matches the reference script byte for byte.

use serde_json::Value;

pub fn py_dumps(v: &Value) -> String {
    let mut out = String::new();
    write_value(v, &mut out);
    out
}

/// Strings as they are, everything else JSON-encoded (the script's `stringify`).
pub fn stringify(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => py_dumps(other),
    }
}

fn write_value(v: &Value, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => write_number(n, out),
        Value::String(s) => write_string(s, out),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_value(item, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            out.push('{');
            for (i, (k, item)) in map.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_string(k, out);
                out.push_str(": ");
                write_value(item, out);
            }
            out.push('}');
        }
    }
}

fn write_number(n: &serde_json::Number, out: &mut String) {
    if let Some(i) = n.as_i64() {
        out.push_str(&i.to_string());
    } else if let Some(u) = n.as_u64() {
        out.push_str(&u.to_string());
    } else if let Some(f) = n.as_f64() {
        // Python repr: shortest round-trip, always with a fractional part or exponent.
        if f.is_finite() && f == f.trunc() && f.abs() < 1e16 {
            out.push_str(&format!("{f:.1}"));
        } else {
            out.push_str(&format!("{f}"));
        }
    }
}

fn write_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}
