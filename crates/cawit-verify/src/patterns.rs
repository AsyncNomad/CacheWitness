//! Built-in race patterns. A scenario names one (`pattern: { stale-set: {...} }`)
//! instead of listing steps; the pattern expands into ordinary steps, so
//! everything a pattern does can also be written by hand. Add patterns with
//! [`Registry::register`].

use std::collections::HashMap;

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_yaml::{Mapping, Value};

use crate::scenario::Scenario;

type Expander = fn(&Scenario, Value) -> Result<Vec<Value>>;

pub struct Registry {
    patterns: HashMap<&'static str, Expander>,
}

impl Registry {
    pub fn builtin() -> Registry {
        let mut r = Registry { patterns: HashMap::new() };
        r.register("stale-set", stale_set);
        r
    }

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
        step("http", &[("app", s(&p.reader)), ("request", s(&p.write)), ("vars", vars(&p.value_var, "${run}-a"))]),
        step("sleep", &[("ms", 50.into())]),
        step("arm", &[("point", s(&p.point)), ("key", opt(p.key.as_deref())), ("as", s("refill"))]),
        step("http", &[("app", s(&p.reader)), ("request", s(&p.read)), ("async", s("held_read"))]),
        step("reached", &[("sync", s("refill")), ("timeout_ms", p.reached_timeout_ms.into())]),
        step("http", &[("app", s(&p.writer)), ("request", s(&p.write)), ("vars", vars(&p.value_var, "${run}-b"))]),
    ];
    if p.hold_ms > 0 {
        steps.push(step("sleep", &[("ms", p.hold_ms.into())]));
    }
    steps.extend([
        step("release", &[("sync", s("refill"))]),
        step("await", &[("task", s("held_read"))]),
        step("sleep", &[("ms", p.settle_ms.into())]),
        step("expect", &[("name", s("not_stale")), ("check", s(not_stale)), ("within_ms", opt(p.within_ms))]),
    ]);
    if !via_app {
        // Reading through the app already refilled on a miss; otherwise read once.
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
        Scenario::parse(&format!("name: t\napps: {{ a: http://x, b: http://y }}\nredis: redis://r\nstate: {state}\npattern: {{ stale-set: {{}} }}\n"))
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
            ["http", "sleep", "arm", "http", "reached", "http", "sleep", "release", "await", "sleep", "expect", "http", "sleep", "expect"]
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
    fn rejects_unknown_pattern_and_params() {
        let sc = scenario("{ cache_key: k, db_query: q }");
        let unknown: Value = serde_yaml::from_str("{ teleport: {} }").unwrap();
        assert!(Registry::builtin().expand(&sc, &unknown).is_err());
        let typo: Value = serde_yaml::from_str("{ stale-set: { reader: a, writer: b, write: w, read: r, hold: 1 } }").unwrap();
        assert!(Registry::builtin().expand(&sc, &typo).is_err());
    }
}
