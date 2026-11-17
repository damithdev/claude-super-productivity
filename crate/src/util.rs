//! Small helpers: paths, JSON files, hook input, and the Python text behaviour the
//! registry and task output keep (`str()`, `json.dumps`, `splitlines`, universal-newline reads).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::Value;

pub fn home() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .expect("neither USERPROFILE nor HOME is set")
}

/// One folder for hooks and Bash runs alike: hooks get CLAUDE_PLUGIN_DATA, skills' Bash calls don't.
pub fn state_dir() -> PathBuf {
    env_nonempty("SP_STATE_DIR").map(PathBuf::from).unwrap_or_else(|| home().join(".claude").join("state"))
}

/// An unset or empty value means unset, the same as Python's `os.environ.get(k) or default`.
pub fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

/// SP_HOOKS=off turns every sp-cli hook into a no-op.
pub fn hooks_off() -> bool {
    std::env::var("SP_HOOKS").is_ok_and(|v| v.eq_ignore_ascii_case("off"))
}

pub fn read_json(path: &Path) -> Result<Value> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_str(text.trim_start_matches('\u{feff}')).with_context(|| format!("parsing {}", path.display()))
}

/// Writes `bytes` to a temp file beside `path`, then renames it over `path`, so a reader sees the
/// old file or the new one, never a half-written one.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| std::io::Error::other(format!("no parent folder for {}", path.display())))?;
    std::fs::create_dir_all(parent)?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = parent.join(format!(".{name}.{}.tmp", std::process::id()));
    let written = std::fs::File::create(&tmp).and_then(|mut f| f.write_all(bytes).and_then(|()| f.sync_all()));
    if let Err(e) = written.and_then(|()| std::fs::rename(&tmp, path)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

pub fn read_stdin() -> String {
    let mut raw = String::new();
    let _ = std::io::stdin().read_to_string(&mut raw);
    raw
}

/// `Path.read_text(errors="replace")`: lossy UTF-8, `\r\n` and `\r` read as `\n`.
pub fn read_text(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    Some(String::from_utf8_lossy(&bytes).replace("\r\n", "\n").replace('\r', "\n"))
}

/// `str.splitlines()`: every Unicode line boundary, no trailing empty line.
pub fn splitlines(s: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut chars = s.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if matches!(c, '\n' | '\r' | '\x0b' | '\x0c' | '\x1c' | '\x1d' | '\x1e' | '\u{85}' | '\u{2028}' | '\u{2029}') {
            lines.push(&s[start..i]);
            let mut end = i + c.len_utf8();
            if c == '\r' && chars.peek().is_some_and(|(_, n)| *n == '\n') {
                chars.next();
                end += 1;
            }
            start = end;
        }
    }
    if start < s.len() {
        lines.push(&s[start..]);
    }
    lines
}

/// `s[:n]`.
pub fn head(s: &str, n: usize) -> &str {
    s.char_indices().nth(n).map_or(s, |(i, _)| &s[..i])
}

/// Python truthiness of a JSON value.
pub fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

/// `str(v)`.
pub fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => repr_value(other),
    }
}

/// `repr(s)`.
fn repr(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') { '"' } else { '\'' };
    let mut out = String::from(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || (0x7f..=0xa0).contains(&(c as u32)) => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// `repr()` of the JSON value as Python would have loaded it.
fn repr_value(v: &Value) -> String {
    match v {
        Value::Null => "None".into(),
        Value::Bool(b) => if *b { "True" } else { "False" }.into(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => repr(s),
        Value::Array(a) => format!("[{}]", a.iter().map(repr_value).collect::<Vec<_>>().join(", ")),
        Value::Object(o) => {
            format!("{{{}}}", o.iter().map(|(k, v)| format!("{}: {}", repr(k), repr_value(v))).collect::<Vec<_>>().join(", "))
        }
    }
}

/// `json.dumps(v)`: `", "` and `": "` separators, non-ASCII escaped, keys in insertion order.
pub fn dumps(v: &Value) -> String {
    match v {
        Value::String(s) => {
            let mut out = String::from('"');
            for c in s.chars() {
                match c {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    '\x08' => out.push_str("\\b"),
                    '\x0c' => out.push_str("\\f"),
                    c if (c as u32) >= 0x20 && c.is_ascii() => out.push(c),
                    c => {
                        let mut buf = [0u16; 2];
                        for unit in c.encode_utf16(&mut buf) {
                            out.push_str(&format!("\\u{unit:04x}"));
                        }
                    }
                }
            }
            out.push('"');
            out
        }
        Value::Array(a) => format!("[{}]", a.iter().map(dumps).collect::<Vec<_>>().join(", ")),
        Value::Object(o) => {
            let items: Vec<String> = o.iter().map(|(k, x)| format!("{}: {}", dumps(&Value::String(k.clone())), dumps(x))).collect();
            format!("{{{}}}", items.join(", "))
        }
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".into(),
        Value::Number(n) => n.to_string(),
    }
}

/// `json.dumps(v, indent=indent)`: one item per line, `","` and `": "` separators, non-ASCII escaped.
pub fn dumps_indent(v: &Value, indent: usize) -> String {
    let mut out = String::new();
    indented(v, indent, 0, &mut out);
    out
}

fn indented(v: &Value, indent: usize, level: usize, out: &mut String) {
    let pad = |l: usize| format!("\n{}", " ".repeat(indent * l));
    match v {
        Value::Array(a) if !a.is_empty() => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                out.push_str(if i > 0 { "," } else { "" });
                out.push_str(&pad(level + 1));
                indented(x, indent, level + 1, out);
            }
            out.push_str(&pad(level));
            out.push(']');
        }
        Value::Object(o) if !o.is_empty() => {
            out.push('{');
            for (i, (k, x)) in o.iter().enumerate() {
                out.push_str(if i > 0 { "," } else { "" });
                out.push_str(&pad(level + 1));
                out.push_str(&dumps(&Value::String(k.clone())));
                out.push_str(": ");
                indented(x, indent, level + 1, out);
            }
            out.push_str(&pad(level));
            out.push('}');
        }
        other => out.push_str(&dumps(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn splitlines_matches_python() {
        assert_eq!(splitlines("a\r\nb\n\nc\n"), vec!["a", "b", "", "c"]);
        assert_eq!(splitlines("a\rb\u{2028}c"), vec!["a", "b", "c"]);
        assert_eq!(splitlines(""), Vec::<&str>::new());
        assert_eq!(splitlines("\n"), vec![""]);
        assert_eq!(splitlines("x\n\n"), vec!["x", ""]);
    }

    #[test]
    fn dumps_matches_python() {
        let v = json!({"b@m": true, "a@m": false, "é": "—"});
        assert_eq!(dumps(&v), r#"{"b@m": true, "a@m": false, "\u00e9": "\u2014"}"#);
        let v = json!({"a": [1, {}], "b": [], "\u{e9}": {"c": null}});
        assert_eq!(dumps_indent(&v, 1), "{\n \"a\": [\n  1,\n  {}\n ],\n \"b\": [],\n \"\\u00e9\": {\n  \"c\": null\n }\n}");
        assert_eq!(dumps_indent(&json!({"k": ["x"]}), 0), "{\n\"k\": [\n\"x\"\n]\n}");
    }

    #[test]
    fn repr_matches_python() {
        assert_eq!(repr("it's"), "\"it's\"");
        assert_eq!(repr("a\nb'\""), "'a\\nb\\'\"'");
        assert_eq!(repr_value(&json!([{"q": "x", "ask": false}])), "[{'q': 'x', 'ask': False}]");
    }
}
