//! Parser for PostgreSQL `test_decoding` output (with include-timestamp).
//! A production version would speak the pgoutput streaming replication
//! protocol; polling test_decoding keeps the prototype small.

use std::collections::BTreeMap;

pub type Row = BTreeMap<String, String>;

#[derive(Debug, Clone)]
pub enum Change {
    Upsert { table: String, row: Row, old: Option<Row> },
    Delete { table: String, old: Row },
}

#[derive(Debug)]
pub enum Line {
    Begin,
    Change(Change),
    Commit { ts: f64 },
    Other,
}

/// Parse `name[type]:value name[type]:'quoted value' ...`
fn parse_cols(s: &str) -> Row {
    let mut row = Row::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && b[i] == b' ' {
            i += 1;
        }
        let name_start = i;
        while i < b.len() && b[i] != b'[' {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        let name = s[name_start..i].to_string();
        while i < b.len() && b[i] != b']' {
            i += 1;
        }
        i += 2; // skip "]:"
        if i > b.len() {
            break;
        }
        let mut val = String::new();
        if i < b.len() && b[i] == b'\'' {
            i += 1;
            loop {
                if i >= b.len() {
                    break;
                }
                if b[i] == b'\'' {
                    if i + 1 < b.len() && b[i + 1] == b'\'' {
                        val.push('\'');
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                // push full utf-8 char
                let ch = s[i..].chars().next().unwrap();
                val.push(ch);
                i += ch.len_utf8();
            }
        } else {
            let st = i;
            while i < b.len() && b[i] != b' ' {
                i += 1;
            }
            val = s[st..i].to_string();
        }
        row.insert(name, val);
    }
    row
}

pub fn parse_line(line: &str) -> Line {
    if line.starts_with("BEGIN") {
        return Line::Begin;
    }
    if let Some(rest) = line.strip_prefix("COMMIT") {
        // COMMIT 735 (at 2026-10-14 17:08:47.738811+09)
        if let Some(p) = rest.find("(at ") {
            let t = rest[p + 4..].trim_end_matches(')');
            if let Ok(dt) = chrono::DateTime::parse_from_str(&format!("{t}00"), "%Y-%m-%d %H:%M:%S%.f%z") {
                return Line::Commit { ts: dt.timestamp_micros() as f64 / 1e6 };
            }
        }
        return Line::Commit { ts: crate::now() };
    }
    if let Some(rest) = line.strip_prefix("table ") {
        // table public.users: UPDATE: old-key: ... new-tuple: ...
        let colon = match rest.find(": ") {
            Some(c) => c,
            None => return Line::Other,
        };
        let table = rest[..colon].rsplit('.').next().unwrap_or("").to_string();
        let body = &rest[colon + 2..];
        if let Some(b) = body.strip_prefix("INSERT: ") {
            return Line::Change(Change::Upsert { table, row: parse_cols(b), old: None });
        }
        if let Some(b) = body.strip_prefix("UPDATE: ") {
            return match b.find("new-tuple: ") {
                Some(p) => {
                    let old = b[..p].strip_prefix("old-key: ").map(parse_cols);
                    Line::Change(Change::Upsert { table, row: parse_cols(&b[p + 11..]), old })
                }
                None => Line::Change(Change::Upsert { table, row: parse_cols(b), old: None }),
            };
        }
        if let Some(b) = body.strip_prefix("DELETE: ") {
            let old = b.strip_prefix("old-key: ").unwrap_or(b);
            return Line::Change(Change::Delete { table, old: parse_cols(old) });
        }
    }
    Line::Other
}
