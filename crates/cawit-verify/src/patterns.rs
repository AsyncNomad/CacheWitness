//! Race patterns: named recipes that expand into ordinary steps.
//!
//! A scenario names one (`pattern: { stale-set: {...} }`) instead of listing
//! steps. Because a pattern only produces steps, everything it does can also
//! be written by hand, and `cawit verify --show-steps` prints the expansion.
//!
//! | Pattern | Race | Hook point |
//! |---|---|---|
//! | `stale-set` | a reader refills with a row read before a newer write committed and invalidated | `refill:after-read`, between the database read and the cache write |
//! | `failed-invalidation` | a write commits but its invalidation fails once | `invalidate:before`, right before the invalidation (armed to fail) |
//! | `invalidate-before-commit` | a write invalidates inside its transaction; a reader refills before the commit | `write:before-commit`, at the end of the transaction |
//!
//! All built-in patterns end with `not_stale` and `refilled` expectations,
//! using the `app_*` checks when `state.app_view` is set and the `cache_*`
//! checks otherwise. Parameters are listed in the
//! [scenario reference](https://github.com/AsyncNomad/CacheWitness/blob/main/docs/scenarios.md#patterns).
//!
//! # Adding a pattern
//!
//! ```
//! use cawit_verify::patterns::Registry;
//! use cawit_verify::Scenario;
//!
//! // `{ read-twice: { app: a, read: read } }`: read twice, then expect.
//! fn read_twice(_sc: &Scenario, p: serde_yaml::Value) -> anyhow::Result<Vec<serde_yaml::Value>> {
//!     let app = p["app"].as_str().unwrap_or("a").to_string();
//!     let read = p["read"].as_str().unwrap_or("read").to_string();
//!     let step = format!("{{ http: {{ app: {app}, request: {read} }} }}");
//!     Ok(vec![
//!         serde_yaml::from_str(&step)?,
//!         serde_yaml::from_str(&step)?,
//!         serde_yaml::from_str("{ expect: { name: fresh, check: app_equals_db } }")?,
//!     ])
//! }
//!
//! let mut patterns = Registry::builtin();
//! patterns.register("read-twice", read_twice);
//! // pass it to the runner with `Options { patterns, ..Options::default() }`
//! ```

use std::collections::HashMap;

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_yaml::{Mapping, Value};

use crate::scenario::Scenario;

/// Expands a pattern's YAML parameters into steps. Gets the scenario so it
/// can choose checks from `state`.
pub type Expander = fn(&Scenario, Value) -> Result<Vec<Value>>;

/// Patterns by name.
pub struct Registry {
    patterns: HashMap<&'static str, Expander>,
}

impl Registry {
    /// The built-in patterns: `stale-set`, `failed-invalidation`, `invalidate-before-commit`.
    pub fn builtin() -> Registry {
        let mut r = Registry { patterns: HashMap::new() };
        r.register("stale-set", stale_set);
        r.register("failed-invalidation", failed_invalidation);
        r.register("invalidate-before-commit", invalidate_before_commit);
        r
    }

    /// Add or replace a pattern.
    pub fn register(&mut self, name: &'static str, expand: Expander) {
        self.patterns.insert(name, expand);
    }

    /// Expand `{ <pattern>: <params> }` into steps.
    pub fn expand(&self, scenario: &Scenario, v: &Value) -> Result<Vec<Value>> {
        let map = v.as_mapping().context("`pattern` must be a mapping like `{ stale-set: {...} }`")?;
        if map.len() != 1 {
            bail!("`pattern` must name exactly one pattern, got {}", map.len());
        }
        let (k, p) = map.iter().next().unwrap();
        let name = k.as_str().context("pattern name must be a string")?;
        let expand = self.patterns.get(name).ok_or_else(|| anyhow!("unknown pattern `{name}`"))?;
        expand(scenario, p.clone()).with_context(|| format!("expanding pattern `{name}`"))
    }
}

fn params<T: DeserializeOwned>(v: Value) -> Result<T> {
    Ok(serde_yaml::from_value(v)?)
}

/// One step `{ kind: { k: v, ... } }`, skipping null values.
fn step(kind: &str, fields: &[(&str, Value)]) -> Value {
    let mut inner = Mapping::new();
    for (k, v) in fields {
        if !v.is_null() {
            inner.insert(Value::from(*k), v.clone());
        }
    }
    let mut outer = Mapping::new();
    outer.insert(Value::from(kind), Value::Mapping(inner));
    Value::Mapping(outer)
}

fn s(x: &str) -> Value {
    Value::from(x)
}

fn opt<T: Into<Value>>(x: Option<T>) -> Value {
    x.map(Into::into).unwrap_or(Value::Null)
}

fn vars(var: &str, value: &str) -> Value {
    let mut m = Mapping::new();
    m.insert(s(var), s(value));
    Value::Mapping(m)
}

/// Checks for "readers do not get a stale value" and "the cache refills",
/// reading through the application when `state.app_view` is set, otherwise
/// the cache entry `state.cache_key`.
fn verdict_checks(scenario: &Scenario) -> Result<(&'static str, &'static str, bool)> {
    let state = scenario.state.as_ref().context("patterns need a `state` section")?;
    match (&state.app_view, &state.cache_key) {
        (Some(_), _) => Ok(("app_equals_db", "app_cached_equals_db", true)),
        (None, Some(_)) => Ok(("cache_not_stale", "cache_equals_db", false)),
        (None, None) => bail!("`state` needs `app_view` or `cache_key`"),
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StaleSet {
    /// App whose refill is held (it also makes the first write).
    reader: String,
    /// App that writes the newer value while the reader is held.
    writer: String,
    /// Request that writes `${<value_var>}`; request that reads (and refills).
    write: String,
    read: String,
    #[serde(default = "value_var")]
    value_var: String,
    #[serde(default = "after_read")]
    point: String,
    /// Only hold the refill of this key.
    #[serde(default)]
    key: Option<String>,
    /// Keep the reader held this long after the newer write (a slow reader).
    #[serde(default)]
    hold_ms: u64,
    /// Wait after the reader finishes before judging (writes that complete
    /// after the response).
    #[serde(default = "hundred")]
    settle_ms: u64,
    /// Bounded-staleness verdict instead of strict (see `expect.within_ms`).
    #[serde(default)]
    within_ms: Option<u64>,
    #[serde(default = "ten_seconds")]
    reached_timeout_ms: u64,
}

fn value_var() -> String {
    "value".into()
}
fn after_read() -> String {
    "refill:after-read".into()
}
fn hundred() -> u64 {
    100
}
fn ten_seconds() -> u64 {
    10_000
}

/// Stale-set: the reader misses and reads the old row, a newer write commits
/// and invalidates, then the reader writes what it read.
///
///   1. write through the reader app (commit, invalidate; clears its local caches)
///   2. hold the reader's refill right after its database read
///   3. write a newer value through the writer app
///   4. optionally keep the reader held (`hold_ms`), then release it
///   5. `not_stale`: readers must get the committed value
///   6. `refilled`: the cache refills with it (a fix must not stop caching)
fn stale_set(scenario: &Scenario, p: Value) -> Result<Vec<Value>> {
    let p: StaleSet = params(p)?;
    let (not_stale, refilled, via_app) = verdict_checks(scenario)?;
    let mut steps = vec![
        step(
            "http",
            &[
                ("app", s(&p.reader)),
                ("request", s(&p.write)),
                ("vars", vars(&p.value_var, "${run}-a")),
            ],
        ),
        step("sleep", &[("ms", 50.into())]),
        step(
            "arm",
            &[("point", s(&p.point)), ("key", opt(p.key.as_deref())), ("as", s("refill"))],
        ),
        step("http", &[("app", s(&p.reader)), ("request", s(&p.read)), ("async", s("held_read"))]),
        step("reached", &[("sync", s("refill")), ("timeout_ms", p.reached_timeout_ms.into())]),
        step(
            "http",
            &[
                ("app", s(&p.writer)),
                ("request", s(&p.write)),
                ("vars", vars(&p.value_var, "${run}-b")),
            ],
        ),
    ];
    if p.hold_ms > 0 {
        steps.push(step("sleep", &[("ms", p.hold_ms.into())]));
    }
    steps.extend([
        step("release", &[("sync", s("refill"))]),
        step("await", &[("task", s("held_read"))]),
        step("sleep", &[("ms", p.settle_ms.into())]),
        step(
            "expect",
            &[("name", s("not_stale")), ("check", s(not_stale)), ("within_ms", opt(p.within_ms))],
        ),
    ]);
    if !via_app {
        // Reading through the app already refilled on a miss; otherwise read once.
        steps.push(step("http", &[("app", s(&p.reader)), ("request", s(&p.read))]));
        steps.push(step("sleep", &[("ms", p.settle_ms.into())]));
    }
    steps.push(step("expect", &[("name", s("refilled")), ("check", s(refilled))]));
    Ok(steps)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FailedInvalidation {
    /// App that writes and reads.
    app: String,
    write: String,
    read: String,
    #[serde(default = "value_var")]
    value_var: String,
    /// Sync point placed right before the invalidation; armed to fail.
    #[serde(default = "before_invalidate")]
    point: String,
    #[serde(default)]
    key: Option<String>,
    #[serde(default = "hundred")]
    settle_ms: u64,
    #[serde(default)]
    within_ms: Option<u64>,
    #[serde(default = "ten_seconds")]
    reached_timeout_ms: u64,
}

fn before_invalidate() -> String {
    "invalidate:before".into()
}

/// Failed invalidation: the write commits but its invalidation fails once
/// (injected at a sync point armed with action `fail`), so the cache keeps
/// the old value unless the code retries or otherwise recovers.
///
///   1. write `a` and read it, so the cache holds `a`
///   2. arm the invalidation point to fail, write `b` (the request may fail)
///   3. `reached` confirms the fault really fired (manipulation check)
///   4. `not_stale`: readers must get `b`
///   5. `refilled`: the cache holds `b` again
fn failed_invalidation(scenario: &Scenario, p: Value) -> Result<Vec<Value>> {
    let p: FailedInvalidation = params(p)?;
    let (not_stale, refilled, via_app) = verdict_checks(scenario)?;
    let mut steps = vec![
        step(
            "http",
            &[
                ("app", s(&p.app)),
                ("request", s(&p.write)),
                ("vars", vars(&p.value_var, "${run}-a")),
            ],
        ),
        step("http", &[("app", s(&p.app)), ("request", s(&p.read))]),
        step("sleep", &[("ms", p.settle_ms.into())]),
        step(
            "arm",
            &[
                ("point", s(&p.point)),
                ("key", opt(p.key.as_deref())),
                ("action", s("fail")),
                ("as", s("fault")),
            ],
        ),
        step(
            "http",
            &[
                ("app", s(&p.app)),
                ("request", s(&p.write)),
                ("vars", vars(&p.value_var, "${run}-b")),
                ("allow_error", true.into()),
            ],
        ),
        step("reached", &[("sync", s("fault")), ("timeout_ms", p.reached_timeout_ms.into())]),
        step("sleep", &[("ms", p.settle_ms.into())]),
        step(
            "expect",
            &[("name", s("not_stale")), ("check", s(not_stale)), ("within_ms", opt(p.within_ms))],
        ),
    ];
    if !via_app {
        steps.push(step("http", &[("app", s(&p.app)), ("request", s(&p.read))]));
        steps.push(step("sleep", &[("ms", p.settle_ms.into())]));
    }
    steps.push(step("expect", &[("name", s("refilled")), ("check", s(refilled))]));
    Ok(steps)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InvalidateBeforeCommit {
    /// App whose write is held just before its commit.
    writer: String,
    /// App that reads (and refills) while the write is uncommitted.
    reader: String,
    write: String,
    read: String,
    #[serde(default = "value_var")]
    value_var: String,
    /// Sync point at the end of the write transaction, before the commit.
    #[serde(default = "before_commit")]
    point: String,
    #[serde(default)]
    key: Option<String>,
    /// Keep the write uncommitted this long before the reader reads (a slow
    /// commit, with the reader arriving late in the window). Pin it when the
    /// code under test relies on timers, e.g. a delayed second invalidation:
    /// a reader that refills after the timer and before the commit is the
    /// order such a fix misses, and without a pinned hold the verdict depends
    /// on how long the steps happened to take.
    #[serde(default)]
    hold_ms: u64,
    #[serde(default = "hundred")]
    settle_ms: u64,
    #[serde(default)]
    within_ms: Option<u64>,
    #[serde(default = "ten_seconds")]
    reached_timeout_ms: u64,
}

fn before_commit() -> String {
    "write:before-commit".into()
}

/// Invalidate before commit: a write that invalidates inside its transaction
/// leaves a window in which a reader refills with the still-committed old
/// value; after the commit nothing removes it.
///
///   1. write `a` and let it commit
///   2. hold the next write (`b`) at the end of its transaction, before commit
///   3. read through the reader app: it misses and refills with `a`
///   4. release the write, which commits `b`
///   5. `not_stale`, `refilled`
///
/// Code that invalidates after the commit passes; this pattern is also a
/// check that a correct implementation is not flagged.
fn invalidate_before_commit(scenario: &Scenario, p: Value) -> Result<Vec<Value>> {
    let p: InvalidateBeforeCommit = params(p)?;
    let (not_stale, refilled, via_app) = verdict_checks(scenario)?;
    let mut steps = vec![
        step(
            "http",
            &[
                ("app", s(&p.writer)),
                ("request", s(&p.write)),
                ("vars", vars(&p.value_var, "${run}-a")),
            ],
        ),
        step("sleep", &[("ms", p.settle_ms.into())]),
        step(
            "arm",
            &[("point", s(&p.point)), ("key", opt(p.key.as_deref())), ("as", s("commit"))],
        ),
        step(
            "http",
            &[
                ("app", s(&p.writer)),
                ("request", s(&p.write)),
                ("vars", vars(&p.value_var, "${run}-b")),
                ("async", s("held_write")),
            ],
        ),
        step("reached", &[("sync", s("commit")), ("timeout_ms", p.reached_timeout_ms.into())]),
        step("sleep", &[("ms", p.hold_ms.into())]),
        step("http", &[("app", s(&p.reader)), ("request", s(&p.read))]),
        step("sleep", &[("ms", p.settle_ms.into())]),
        step("release", &[("sync", s("commit"))]),
        step("await", &[("task", s("held_write"))]),
        step("sleep", &[("ms", p.settle_ms.into())]),
        step(
            "expect",
            &[("name", s("not_stale")), ("check", s(not_stale)), ("within_ms", opt(p.within_ms))],
        ),
    ];
    if !via_app {
        steps.push(step("http", &[("app", s(&p.reader)), ("request", s(&p.read))]));
        steps.push(step("sleep", &[("ms", p.settle_ms.into())]));
    }
    steps.push(step("expect", &[("name", s("refilled")), ("check", s(refilled))]));
    Ok(steps)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scenario(state: &str) -> Scenario {
        Scenario::parse(&format!(
            "name: t\napps: {{ a: http://x, b: http://y }}\nredis: redis://r\nstate: {state}\npattern: {{ stale-set: {{}} }}\n"
        ))
        .unwrap()
    }

    #[test]
    fn stale_set_expands_into_buildable_steps() {
        let sc = scenario("{ cache_key: k, db_query: q }");
        let p: Value = serde_yaml::from_str("{ stale-set: { reader: a, writer: b, write: w, read: r, hold_ms: 1000 } }").unwrap();
        let steps = Registry::builtin().expand(&sc, &p).unwrap();
        let reg = crate::steps::Registry::builtin();
        let kinds: Vec<&str> = steps.iter().map(|v| reg.build(v).unwrap().kind()).collect();
        assert_eq!(
            kinds,
            [
                "http", "sleep", "arm", "http", "reached", "http", "sleep", "release", "await", "sleep", "expect", "http", "sleep",
                "expect"
            ]
        );
        let text = serde_yaml::to_string(&steps).unwrap();
        assert!(text.contains("cache_not_stale") && !text.contains("within_ms") && !text.contains("key:"));
    }

    #[test]
    fn app_view_switches_checks_and_skips_extra_read() {
        let sc = scenario("{ db_query: q, app_view: { app: a, request: r, field: /x } }");
        let p: Value = serde_yaml::from_str("{ stale-set: { reader: a, writer: b, write: w, read: r, within_ms: 500 } }").unwrap();
        let text = serde_yaml::to_string(&Registry::builtin().expand(&sc, &p).unwrap()).unwrap();
        assert!(text.contains("app_equals_db") && text.contains("app_cached_equals_db") && text.contains("within_ms: 500"));
    }

    #[test]
    fn failed_invalidation_arms_a_fault_and_allows_the_write_to_fail() {
        let sc = scenario("{ db_query: q, app_view: { app: a, request: r, field: /x } }");
        let p: Value = serde_yaml::from_str("{ failed-invalidation: { app: a, write: w, read: r } }").unwrap();
        let steps = Registry::builtin().expand(&sc, &p).unwrap();
        let reg = crate::steps::Registry::builtin();
        for v in &steps {
            reg.build(v).unwrap();
        }
        let text = serde_yaml::to_string(&steps).unwrap();
        assert!(text.contains("action: fail") && text.contains("allow_error: true") && text.contains("invalidate:before"));
    }

    #[test]
    fn invalidate_before_commit_holds_the_write_and_reads_in_between() {
        let sc = scenario("{ cache_key: k, db_query: q }");
        let p: Value = serde_yaml::from_str("{ invalidate-before-commit: { writer: a, reader: b, write: w, read: r } }").unwrap();
        let steps = Registry::builtin().expand(&sc, &p).unwrap();
        let reg = crate::steps::Registry::builtin();
        let kinds: Vec<&str> = steps.iter().map(|v| reg.build(v).unwrap().kind()).collect();
        assert_eq!(&kinds[2..8], ["arm", "http", "reached", "sleep", "http", "sleep"]);
        assert!(serde_yaml::to_string(&steps).unwrap().contains("write:before-commit"));
    }

    #[test]
    fn rejects_unknown_pattern_and_params() {
        let sc = scenario("{ cache_key: k, db_query: q }");
        let unknown: Value = serde_yaml::from_str("{ teleport: {} }").unwrap();
        assert!(Registry::builtin().expand(&sc, &unknown).is_err());
        let typo: Value = serde_yaml::from_str("{ stale-set: { reader: a, writer: b, write: w, read: r, hold: 1 } }").unwrap();
        assert!(Registry::builtin().expand(&sc, &typo).is_err());
    }
}
