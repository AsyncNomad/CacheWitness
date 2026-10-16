//! Verdict checks used by `expect` steps. Each compares the cache entry
//! named in the scenario's `state` with the database. Register more with
//! [`Registry::register`].

use std::collections::HashMap;

use anyhow::{Context, Result, bail};
use cawit_resp::Frame;

use crate::steps::{BoxFut, RunCtx};

pub struct CheckResult {
    pub pass: bool,
    pub detail: String,
}

pub trait Check: Send + Sync {
    fn eval<'a>(&'a self, ctx: &'a mut RunCtx<'_>) -> BoxFut<'a, Result<CheckResult>>;
}

pub struct Registry {
    checks: HashMap<&'static str, Box<dyn Check>>,
}

impl Registry {
    /// Built-ins: `cache_not_stale` (absent or equal to the database),
    /// `cache_equals_db` (present and equal), `cache_absent`.
    pub fn builtin() -> Registry {
        let mut r = Registry { checks: HashMap::new() };
        r.register("cache_not_stale", Box::new(Compare(|cache, db| cache.is_none() || cache.as_deref() == Some(db))));
        r.register("cache_equals_db", Box::new(Compare(|cache, db| cache.as_deref() == Some(db))));
        r.register("cache_absent", Box::new(Compare(|cache, _db| cache.is_none())));
        r
    }

    pub fn register(&mut self, name: &'static str, check: Box<dyn Check>) {
        self.checks.insert(name, check);
    }

    pub fn get(&self, name: &str) -> Option<&dyn Check> {
        self.checks.get(name).map(|b| b.as_ref())
    }
}

/// Cache value (None if absent) and database value as text.
pub async fn observe(ctx: &mut RunCtx<'_>) -> Result<(Option<String>, String)> {
    let state = ctx.scenario.state.as_ref().context("this check needs a `state` section in the scenario")?;
    let reply = ctx.redis.cmd(&["GET", &state.cache_key]).await?;
    let raw = match reply {
        Frame::Bulk(Some(b)) => Some(String::from_utf8_lossy(&b).into_owned()),
        f if f.is_nil() => None,
        Frame::Error(e) => bail!("reading {}: {e}", state.cache_key),
        other => bail!("unexpected reply for {}: {other:?}", state.cache_key),
    };
    let cache = match (raw, &state.cache_field) {
        (Some(text), Some(field)) => {
            let v: serde_json::Value = serde_json::from_str(&text).with_context(|| format!("cache value of {} is not JSON", state.cache_key))?;
            Some(match v.get(field) {
                Some(serde_json::Value::String(s)) => s.clone(),
                Some(other) => other.to_string(),
                None => bail!("cache value has no field `{field}`"),
            })
        }
        (raw, _) => raw,
    };
    let pg = ctx.pg.context("this check needs `postgres` in the scenario")?;
    let row = pg.query_one(state.db_query.as_str(), &[]).await.context("running state.db_query")?;
    let db: String = row.try_get(0).context("state.db_query must return one text column (cast with ::text)")?;
    Ok((cache, db))
}

struct Compare(fn(&Option<String>, &str) -> bool);

impl Check for Compare {
    fn eval<'a>(&'a self, ctx: &'a mut RunCtx<'_>) -> BoxFut<'a, Result<CheckResult>> {
        Box::pin(async move {
            let (cache, db) = observe(ctx).await?;
            let pass = (self.0)(&cache, &db);
            Ok(CheckResult { pass, detail: format!("cache={} db={db}", cache.as_deref().unwrap_or("<absent>")) })
        })
    }
}
