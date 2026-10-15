//! CacheWitness prototype: a Redis (RESP) proxy + PostgreSQL logical decoding
//! reader that detects cache entries inconsistent with the database and
//! explains the likely cause from the observed event timeline.

mod resp;
mod wal;

use resp::Frame;
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::io::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use wal::{Change, Line, Row};

pub fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_micros() as f64 / 1e6
}

// ---------------------------------------------------------------- config
#[derive(Clone)]
struct Config {
    listen: String,
    upstream: String,
    pg: String,
    slot: String,
    table: String,
    pk: String,
    prefix: String,
    grace: f64,       // seconds a mismatch may persist before it is a violation
    ttl_ok: f64,      // stale entries whose TTL is <= this are "bounded by TTL"
    ibc_window: f64,  // DEL within this window before commit counts as "before commit"
    out: String,
}

fn env(k: &str, d: &str) -> String {
    std::env::var(k).unwrap_or_else(|_| d.to_string())
}

// ---------------------------------------------------------------- state
#[derive(Debug, Clone)]
enum EvKind {
    GetHit,
    GetMiss,
    Set { val: Vec<u8>, ttl: Option<f64> },
    Del { ok: bool, err: Option<String> },
}

#[derive(Debug, Clone)]
struct Ev {
    ts: f64,
    conn: u64,
    kind: EvKind,
}

#[derive(Debug, Clone)]
struct Version {
    row: Option<Row>, // None = deleted
    ts: f64,          // commit timestamp (NEG_INFINITY = baseline before observation)
}

#[derive(Default)]
struct KeyState {
    versions: Vec<Version>,
    events: VecDeque<Ev>,
    reported_for: Option<usize>,
}

#[derive(Default)]
struct State {
    keys: HashMap<String, KeyState>,
    checks: Vec<(f64, String)>,
    stats: HashMap<&'static str, u64>,
}

type Shared = Arc<Mutex<State>>;

fn bump(st: &mut State, k: &'static str) {
    *st.stats.entry(k).or_default() += 1;
}

// ---------------------------------------------------------------- value matching
fn json_scalar(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// A cached JSON object "matches" a row when every overlapping column is equal.
fn matches(val: &[u8], row: &Row) -> bool {
    let Ok(Value::Object(m)) = serde_json::from_slice::<Value>(val) else { return false };
    let mut overlap = 0;
    for (k, v) in &m {
        if let Some(rv) = row.get(k) {
            overlap += 1;
            if json_scalar(v) != *rv {
                return false;
            }
        }
    }
    overlap > 0
}

/// Index of the newest version the cached value corresponds to.
fn match_version(val: &[u8], versions: &[Version]) -> Option<usize> {
    versions.iter().enumerate().rev().find(|(_, v)| v.row.as_ref().is_some_and(|r| matches(val, r))).map(|(i, _)| i)
}

// ---------------------------------------------------------------- proxy
async fn handle_client(client: TcpStream, cfg: Arc<Config>, st: Shared, conn: u64) {
    let Ok(upstream) = TcpStream::connect(&cfg.upstream).await else { return };
    let _ = client.set_nodelay(true);
    let _ = upstream.set_nodelay(true);
    if std::env::var("CW_PASSTHROUGH").is_ok() {
        let (mut c, mut u) = (client, upstream);
        let _ = tokio::io::copy_bidirectional(&mut c, &mut u).await;
        return;
    }
    let (mut cr, mut cw) = client.into_split();
    let (mut ur, mut uw) = upstream.into_split();
    let pending: Arc<Mutex<VecDeque<(f64, Vec<Vec<u8>>)>>> = Arc::default();

    let p1 = pending.clone();
    let c2u = async move {
        let mut buf = vec![0u8; 64 * 1024];
        let mut acc: Vec<u8> = Vec::new();
        let mut parse_ok = true;
        loop {
            let n = match cr.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            if parse_ok {
                acc.extend_from_slice(&buf[..n]);
                let ts = now();
                let mut off = 0;
                loop {
                    match resp::parse(&acc[off..]) {
                        Ok(Some((Frame::Array(Some(items)), used))) => {
                            let args: Vec<Vec<u8>> = items.iter().filter_map(|f| f.as_bytes().map(|b| b.to_vec())).collect();
                            p1.lock().unwrap().push_back((ts, args));
                            off += used;
                        }
                        Ok(Some((_, used))) => off += used,
                        Ok(None) => break,
                        Err(_) => {
                            parse_ok = false; // unknown protocol: keep proxying, stop observing
                            break;
                        }
                    }
                }
                acc.drain(..off.min(acc.len()));
            }
            if uw.write_all(&buf[..n]).await.is_err() {
                break;
            }
        }
    };

    let p2 = pending.clone();
    let cfg2 = cfg.clone();
    let u2c = async move {
        let mut buf = vec![0u8; 64 * 1024];
        let mut acc: Vec<u8> = Vec::new();
        loop {
            let n = match ur.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            if cw.write_all(&buf[..n]).await.is_err() {
                break;
            }
            acc.extend_from_slice(&buf[..n]);
            let mut off = 0;
            loop {
                match resp::parse(&acc[off..]) {
                    Ok(Some((Frame::Push(_), used))) => off += used, // out-of-band, no request
                    Ok(Some((frame, used))) => {
                        off += used;
                        let req = p2.lock().unwrap().pop_front();
                        if let Some((ts, args)) = req {
                            observe(&cfg2, &st, conn, ts, &args, &frame);
                        }
                    }
                    Ok(None) => break,
                    Err(_) => {
                        off = acc.len();
                        break;
                    }
                }
            }
            acc.drain(..off.min(acc.len()));
        }
    };
    tokio::join!(c2u, u2c);
}

fn observe(cfg: &Config, st: &Shared, conn: u64, ts: f64, args: &[Vec<u8>], reply: &Frame) {
    if args.len() < 2 {
        return;
    }
    let cmd = String::from_utf8_lossy(&args[0]).to_ascii_uppercase();
    let key = String::from_utf8_lossy(&args[1]).to_string();
    if !key.starts_with(&cfg.prefix) {
        return;
    }
    let kind = match cmd.as_str() {
        "GET" => match reply {
            Frame::Bulk(Some(_)) => EvKind::GetHit,
            _ => EvKind::GetMiss,
        },
        "SET" | "SETEX" | "PSETEX" => {
            if let Frame::Error(_) = reply {
                return;
            }
            let (val, mut ttl) = match cmd.as_str() {
                "SET" if args.len() >= 3 => (args[2].clone(), None),
                "SETEX" if args.len() >= 4 => (args[3].clone(), String::from_utf8_lossy(&args[2]).parse::<f64>().ok()),
                "PSETEX" if args.len() >= 4 => (args[3].clone(), String::from_utf8_lossy(&args[2]).parse::<f64>().ok().map(|x| x / 1000.0)),
                _ => return,
            };
            if cmd == "SET" {
                let mut i = 3;
                while i + 1 < args.len() {
                    let opt = String::from_utf8_lossy(&args[i]).to_ascii_uppercase();
                    let n = String::from_utf8_lossy(&args[i + 1]).parse::<f64>().ok();
                    match opt.as_str() {
                        "EX" => ttl = n,
                        "PX" => ttl = n.map(|x| x / 1000.0),
                        _ => {}
                    }
                    i += 1;
                }
            }
            EvKind::Set { val, ttl }
        }
        "DEL" | "UNLINK" => match reply {
            Frame::Error(e) => EvKind::Del { ok: false, err: Some(e.clone()) },
            _ => EvKind::Del { ok: true, err: None },
        },
        _ => return,
    };
    let mut s = st.lock().unwrap();
    bump(&mut s, "cache_events");
    let mut stale_set = false;
    {
        let ks = s.keys.entry(key.clone()).or_default();
        if let EvKind::Set { val, .. } = &kind {
            // Writing a value that is older than the newest committed version?
            if let Some(last) = ks.versions.last() {
                if last.ts <= ts {
                    let mv = match_version(val, &ks.versions);
                    stale_set = matches!(mv, Some(i) if i < ks.versions.len() - 1);
                }
            }
        }
        ks.events.push_back(Ev { ts, conn, kind });
        if ks.events.len() > 256 {
            ks.events.pop_front();
        }
    }
    if stale_set {
        bump(&mut s, "stale_sets_seen");
        s.checks.push((ts + cfg.grace, key));
    }
}

// ---------------------------------------------------------------- WAL reader
async fn wal_loop(cfg: Arc<Config>, st: Shared) {
    let (client, connection) = tokio_postgres::connect(&cfg.pg, tokio_postgres::NoTls).await.expect("pg connect");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let q = format!("select data from pg_logical_slot_get_changes('{}', NULL, NULL, 'include-timestamp', 'on')", cfg.slot);
    let mut txn: Vec<Change> = Vec::new();
    loop {
        let rows = match client.query(q.as_str(), &[]).await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("wal error: {e}");
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                continue;
            }
        };
        let empty = rows.is_empty();
        for r in rows {
            let data: String = r.get(0);
            match wal::parse_line(&data) {
                Line::Begin => txn.clear(),
                Line::Change(c) => txn.push(c),
                Line::Commit { ts } => {
                    let mut s = st.lock().unwrap();
                    for c in txn.drain(..) {
                        let (table, row, old) = match c {
                            Change::Upsert { table, row, old } => (table, Some(row), old),
                            Change::Delete { table, old } => (table, None, Some(old)),
                        };
                        if table != cfg.table {
                            continue;
                        }
                        let pkv = row.as_ref().or(old.as_ref()).and_then(|r| r.get(&cfg.pk)).cloned().unwrap_or_default();
                        let key = format!("{}{}", cfg.prefix, pkv);
                        let ks = s.keys.entry(key.clone()).or_default();
                        if ks.versions.is_empty() {
                            if let Some(o) = &old {
                                ks.versions.push(Version { row: Some(o.clone()), ts: f64::NEG_INFINITY });
                            }
                        }
                        ks.versions.push(Version { row, ts });
                        if ks.versions.len() > 64 {
                            ks.versions.remove(0);
                            ks.reported_for = ks.reported_for.and_then(|i| i.checked_sub(1));
                        }
                        bump(&mut s, "db_changes");
                        s.checks.push((ts + cfg.grace, key));
                    }
                }
                Line::Other => {}
            }
        }
        if empty {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }
}

// ---------------------------------------------------------------- checker
async fn redis_get(stream: &mut TcpStream, key: &str) -> Option<Vec<u8>> {
    stream.write_all(&resp::encode(&[b"GET", key.as_bytes()])).await.ok()?;
    let mut acc = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = stream.read(&mut buf).await.ok()?;
        if n == 0 {
            return None;
        }
        acc.extend_from_slice(&buf[..n]);
        if let Ok(Some((f, _))) = resp::parse(&acc) {
            return match f {
                Frame::Bulk(Some(b)) => Some(b),
                _ => None,
            };
        }
    }
}

fn fmt_ev(e: &Ev, t0: f64) -> Value {
    let rel = ((e.ts - t0) * 1000.0 * 10.0).round() / 10.0;
    let what = match &e.kind {
        EvKind::GetHit => "GET hit".to_string(),
        EvKind::GetMiss => "GET miss".to_string(),
        EvKind::Set { val, ttl } => format!("SET {}{}", String::from_utf8_lossy(val), ttl.map(|t| format!(" (ttl {t}s)")).unwrap_or_default()),
        EvKind::Del { ok: true, .. } => "DEL".to_string(),
        EvKind::Del { ok: false, err } => format!("DEL FAILED: {}", err.clone().unwrap_or_default()),
    };
    json!({"t_ms_vs_commit": rel, "conn": e.conn, "op": what})
}

/// Explain why `key` holds a value matching version `mv` (or unknown) while
/// `latest` is the newest committed version.
fn classify(cfg: &Config, ks: &KeyState, val: &[u8], mv: Option<usize>) -> (String, String, String) {
    let latest = ks.versions.len() - 1;
    let tc = ks.versions[latest].ts;
    // the SET that put this exact value into the cache
    let last_set = ks.events.iter().rev().find(|e| matches!(&e.kind, EvKind::Set { val: v, .. } if v.as_slice() == val));
    let dels: Vec<&Ev> = ks.events.iter().filter(|e| matches!(e.kind, EvKind::Del { .. })).collect();
    let ok_del_after_commit = dels.iter().find(|e| e.ts >= tc && matches!(e.kind, EvKind::Del { ok: true, .. }));
    let failed_del_after_commit = dels.iter().find(|e| e.ts >= tc - cfg.ibc_window && matches!(e.kind, EvKind::Del { ok: false, .. }));
    let ok_del_before_commit = dels.iter().rev().find(|e| e.ts < tc && e.ts >= tc - cfg.ibc_window && matches!(e.kind, EvKind::Del { ok: true, .. }));
    let ver = |i: Option<usize>| i.map(|i| format!("v{i}")).unwrap_or("unknown".into());

    if let Some(EvKind::Set { ttl: Some(t), .. }) = last_set.map(|e| &e.kind) {
        if *t <= cfg.ttl_ok && ok_del_after_commit.is_none() {
            return ("info".into(), "BOUNDED_BY_TTL".into(),
                format!("cached {} is stale, but it was written with ttl {t}s (<= allowed {}s) and will self-heal", ver(mv), cfg.ttl_ok));
        }
    }
    if let Some(f) = failed_del_after_commit {
        if let EvKind::Del { err, .. } = &f.kind {
            return ("error".into(), "INVALIDATION_FAILED".into(),
                format!("DB committed v{latest}; the app tried to DEL the key but Redis rejected it ({}) and it was not retried", err.clone().unwrap_or_default()));
        }
    }
    match last_set {
        Some(s) if s.ts >= tc => {
            if let Some(d) = ok_del_after_commit.filter(|d| d.ts <= s.ts) {
                let reader_miss = ks.events.iter().rev().find(|e| e.conn == s.conn && e.ts < s.ts && matches!(e.kind, EvKind::GetMiss));
                if let Some(m) = reader_miss.filter(|m| m.ts >= d.ts) {
                    return ("error".into(), "STALE_READ_AFTER_INVALIDATION".into(),
                        format!("v{latest} committed and DEL ran at {:+.1}ms; conn {} missed at {:+.1}ms (after the DEL) yet filled old {} at {:+.1}ms -> it read stale data after the commit, typically a lagging read replica",
                            (d.ts - tc) * 1000.0, s.conn, (m.ts - tc) * 1000.0, ver(mv), (s.ts - tc) * 1000.0));
                }
                let since = reader_miss.map(|m| format!(" (that reader's cache miss was at {:+.1}ms, i.e. it read the DB before the commit)", (m.ts - tc) * 1000.0)).unwrap_or_default();
                ("error".into(), "STALE_SET_RACE".into(),
                    format!("v{latest} committed and DEL ran at {:+.1}ms, but conn {} then wrote back old {} at {:+.1}ms{since}",
                        (d.ts - tc) * 1000.0, s.conn, ver(mv), (s.ts - tc) * 1000.0))
            } else if let Some(d) = ok_del_before_commit {
                ("error".into(), "INVALIDATE_BEFORE_COMMIT".into(),
                    format!("DEL ran {:.1}ms BEFORE the commit of v{latest}; conn {} refilled old {} at {:+.1}ms", (tc - d.ts) * 1000.0, s.conn, ver(mv), (s.ts - tc) * 1000.0))
            } else {
                ("error".into(), "STALE_FILL_AFTER_COMMIT".into(),
                    format!("conn {} wrote old {} {:.1}ms after v{latest} committed and no invalidation followed (read from a lagging replica, or a read that began before the commit)", s.conn, ver(mv), (s.ts - tc) * 1000.0))
            }
        }
        Some(s) => {
            if let Some(d) = ok_del_before_commit.filter(|d| d.ts <= s.ts) {
                ("error".into(), "INVALIDATE_BEFORE_COMMIT".into(),
                    format!("DEL ran {:.1}ms BEFORE the commit of v{latest}; conn {} refilled the not-yet-updated {} at {:+.1}ms and no DEL followed the commit",
                        (tc - d.ts) * 1000.0, s.conn, ver(mv), (s.ts - tc) * 1000.0))
            } else {
                ("error".into(), "MISSING_INVALIDATION".into(),
                    format!("v{latest} committed but no DEL/SET for this key was observed afterwards; cache still holds {} written at {:+.1}ms", ver(mv), (s.ts - tc) * 1000.0))
            }
        }
        None => ("error".into(), "MISSING_INVALIDATION".into(),
            format!("v{latest} committed but no DEL/SET for this key was observed afterwards; the stale {} was written before observation began", ver(mv))),
    }
}

async fn checker_loop(cfg: Arc<Config>, st: Shared) {
    let mut redis = TcpStream::connect(&cfg.upstream).await.expect("redis connect");
    let mut out = std::fs::OpenOptions::new().create(true).append(true).open(&cfg.out).unwrap();
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let t = now();
        let due: Vec<String> = {
            let mut s = st.lock().unwrap();
            let (d, keep): (Vec<_>, Vec<_>) = s.checks.drain(..).partition(|(at, _)| *at <= t);
            s.checks = keep;
            let mut ks: Vec<String> = d.into_iter().map(|(_, k)| k).collect();
            ks.sort();
            ks.dedup();
            ks
        };
        for key in due {
            let val = redis_get(&mut redis, &key).await;
            let mut s = st.lock().unwrap();
            bump(&mut s, "checks");
            let Some(val) = val else { continue };
            let Some(ks) = s.keys.get_mut(&key) else { continue };
            if ks.versions.is_empty() {
                continue;
            }
            let latest = ks.versions.len() - 1;
            let mv = match_version(&val, &ks.versions);
            if mv == Some(latest) || ks.reported_for == Some(latest) {
                continue;
            }
            if mv.is_none() {
                // value not in known history: likely newer than our WAL position. recheck later.
                s.checks.push((t + cfg.grace, key));
                bump(&mut s, "unknown_rechecks");
                continue;
            }
            // newest commit is still inside its grace window? check again later
            if ks.versions[latest].ts + cfg.grace > t {
                continue;
            }
            ks.reported_for = Some(latest);
            let (severity, cause, explanation) = classify(&cfg, ks, &val, mv);
            let tc = ks.versions[latest].ts;
            let timeline: Vec<Value> = ks.events.iter().filter(|e| e.ts >= tc - 1.0).map(|e| fmt_ev(e, tc)).collect();
            let rec = json!({
                "key": key, "severity": severity, "cause": cause, "explanation": explanation,
                "db_latest": ks.versions[latest].row, "cached": String::from_utf8_lossy(&val),
                "cached_matches": mv.map(|i| format!("v{i}")), "detected_ms_after_commit": ((t - tc) * 1000.0).round(),
                "timeline": timeline,
            });
            let sev: &'static str = if severity == "info" { "info_reports" } else { "violations" };
            bump(&mut s, sev);
            writeln!(out, "{rec}").ok();
        }
    }
}

// ---------------------------------------------------------------- main
#[tokio::main]
async fn main() {
    let cfg = Arc::new(Config {
        listen: env("CW_LISTEN", "127.0.0.1:6380"),
        upstream: env("CW_UPSTREAM", "127.0.0.1:6379"),
        pg: env("CW_PG", "host=127.0.0.1 user=postgres dbname=app"),
        slot: env("CW_SLOT", "cw"),
        table: env("CW_TABLE", "users"),
        pk: env("CW_PK", "id"),
        prefix: env("CW_PREFIX", "user:"),
        grace: env("CW_GRACE_MS", "300").parse::<f64>().unwrap() / 1000.0,
        ttl_ok: env("CW_TTL_OK_S", "5").parse().unwrap(),
        ibc_window: env("CW_IBC_WINDOW_MS", "1000").parse::<f64>().unwrap() / 1000.0,
        out: env("CW_OUT", "violations.jsonl"),
    });
    let st: Shared = Arc::default();
    tokio::spawn(wal_loop(cfg.clone(), st.clone()));
    tokio::spawn(checker_loop(cfg.clone(), st.clone()));
    {
        let st = st.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                let s = st.lock().unwrap();
                let mut v: Vec<_> = s.stats.iter().collect();
                v.sort();
                eprintln!("stats {v:?}");
            }
        });
    }
    let listener = TcpListener::bind(&cfg.listen).await.expect("bind");
    eprintln!("cachewitness: proxy {} -> {}, watching {}.{} as {}*", cfg.listen, cfg.upstream, cfg.table, cfg.pk, cfg.prefix);
    let ids = AtomicU64::new(1);
    loop {
        let (sock, _) = listener.accept().await.unwrap();
        let id = ids.fetch_add(1, Ordering::Relaxed);
        tokio::spawn(handle_client(sock, cfg.clone(), st.clone(), id));
    }
}
