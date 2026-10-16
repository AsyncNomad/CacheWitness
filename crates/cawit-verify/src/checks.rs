//! Checks: the verdicts evaluated by `expect` steps.
//!
//! | Check | Reads | Passes when |
//! |---|---|---|
//! | `cache_not_stale` | `state.cache_key` | the entry is absent or equals the database |
//! | `cache_equals_db` | `state.cache_key` | the entry exists and equals the database |
//! | `cache_absent` | `state.cache_key` | the entry is absent |
//! | `app_equals_db` | `state.app_view` | what the application serves equals the database |
//! | `app_cached_equals_db` | `state.app_view` | that, and the response was a cache hit (`hit_header`) |
//!
//! The database value is the first column of the first row of
//! `state.db_query`, as text.
//!
//! # Adding a check
//!
//! ```no_run
//! use cawit_verify::checks::{Check, CheckResult, observe};
//! use cawit_verify::runner::Options;
//! use cawit_verify::steps::{BoxFut, RunCtx};
//!
//! /// Passes when the cache entry is absent or holds valid JSON.
//! struct CacheIsJson;
//!
//! impl Check for CacheIsJson {
//!     fn eval<'a>(&'a self, ctx: &'a mut RunCtx<'_>) -> BoxFut<'a, anyhow::Result<CheckResult>> {
//!         Box::pin(async move {
//!             let (cache, _db) = observe(ctx).await?;
//!             let pass = cache.as_deref().is_none_or(|v| serde_json::from_str::<serde_json::Value>(v).is_ok());
//!             Ok(CheckResult { pass, detail: format!("cache={cache:?}") })
//!         })
//!     }
//! }
//!
//! let mut opts = Options::default();
//! opts.checks.register("cache_is_json", Box::new(CacheIsJson));
//! ```

use std::collections::HashMap;

use anyhow::{Context, Result, bail};
use cawit_resp::Frame;

use crate::steps::{BoxFut, RunCtx};

/// The result of evaluating a check.
pub struct CheckResult {
    /// Whether the check passed.
    pub pass: bool,
    /// What was compared, shown in output and reports.
    pub detail: String,
}

/// A verdict. See the [module documentation](self) for an example.
pub trait Check: Send + Sync {
    /// Evaluate against the current state. `Err` ends the run as `ERROR`.
    fn eval<'a>(&'a self, ctx: &'a mut RunCtx<'_>) -> BoxFut<'a, Result<CheckResult>>;
}

/// Checks by name.
pub struct Registry {
    checks: HashMap<&'static str, Box<dyn Check>>,
}

impl Registry {
    /// Built-ins reading the cache entry: `cache_not_stale` (absent or equal
    /// to the database), `cache_equals_db` (present and equal), `cache_absent`.
    /// Built-ins reading through the application (`state.app_view`):
    /// `app_equals_db` (what it serves equals the database) and
    /// `app_cached_equals_db` (that, and it was served from the cache).
    pub fn builtin() -> Registry {
        let mut r = Registry { checks: HashMap::new() };
        r.register(
            "cache_not_stale",
            Box::new(Compare(|cache, db| cache.is_none() || cache.as_deref() == Some(db))),
        );
        r.register("cache_equals_db", Box::new(Compare(|cache, db| cache.as_deref() == Some(db))));
        r.register("cache_absent", Box::new(Compare(|cache, _db| cache.is_none())));
        r.register("app_equals_db", Box::new(AppView { need_hit: false }));
        r.register("app_cached_equals_db", Box::new(AppView { need_hit: true }));
        r
    }

    /// Add or replace a check.
    pub fn register(&mut self, name: &'static str, check: Box<dyn Check>) {
        self.checks.insert(name, check);
    }

    /// Look up a check by name.
    pub fn get(&self, name: &str) -> Option<&dyn Check> {
        self.checks.get(name).map(|b| b.as_ref())
    }
}

/// The cache entry `state.cache_key` (its `cache_field` if set; `None` if
/// absent) and the database value, as text.
pub async fn observe(ctx: &mut RunCtx<'_>) -> Result<(Option<String>, String)> {
    let state = ctx
        .scenario
        .state
        .as_ref()
        .context("this check needs a `state` section in the scenario")?;
    let key = state.cache_key.as_deref().context("this check needs `state.cache_key`")?;
    let reply = ctx.redis.cmd(&["GET", key]).await?;
    let raw = match reply {
        Frame::Bulk(Some(b)) => Some(String::from_utf8_lossy(&b).into_owned()),
        f if f.is_nil() => None,
        Frame::Error(e) => bail!("reading {key}: {e}"),
        other => bail!("unexpected reply for {key}: {other:?}"),
    };
    let cache = match (raw, &state.cache_field) {
        (Some(text), Some(field)) => {
            let v: serde_json::Value = serde_json::from_str(&text).with_context(|| format!("cache value of {key} is not JSON"))?;
            Some(json_text(
                v.get(field).with_context(|| format!("cache value has no field `{field}`"))?,
            ))
        }
        (raw, _) => raw,
    };
    Ok((cache, db_value(ctx).await?))
}

async fn db_value(ctx: &RunCtx<'_>) -> Result<String> {
    let state = ctx
        .scenario
        .state
        .as_ref()
        .context("this check needs a `state` section in the scenario")?;
    let pg = ctx.pg.context("this check needs `postgres` in the scenario")?;
    let row = pg.query_one(state.db_query.as_str(), &[]).await.context("running state.db_query")?;
    row.try_get(0)
        .context("state.db_query must return one text column (cast with ::text)")
}

fn json_text(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// What the application serves for `state.app_view`, and whether it came from the cache.
pub async fn observe_app(ctx: &mut RunCtx<'_>) -> Result<(String, Option<bool>)> {
    let state = ctx
        .scenario
        .state
        .as_ref()
        .context("this check needs a `state` section in the scenario")?;
    let view = state.app_view.as_ref().context("this check needs `state.app_view`")?;
    let resp = crate::steps::prepare(ctx, &view.app, &view.request, &Default::default())?
        .send()
        .await?;
    let body: serde_json::Value = serde_json::from_slice(&resp.body).context("app_view response is not JSON")?;
    let value = body
        .pointer(&view.field)
        .with_context(|| format!("app_view response has no `{}`", view.field))?;
    let hit = view.hit_header.as_ref().map(|h| resp.header(h) == Some(view.hit_value.as_str()));
    Ok((json_text(value), hit))
}

struct AppView {
    need_hit: bool,
}

impl Check for AppView {
    fn eval<'a>(&'a self, ctx: &'a mut RunCtx<'_>) -> BoxFut<'a, Result<CheckResult>> {
        Box::pin(async move {
            if self.need_hit {
                let view = ctx.scenario.state.as_ref().and_then(|s| s.app_view.as_ref());
                if view.is_some_and(|v| v.hit_header.is_none()) {
                    bail!("app_cached_equals_db needs `state.app_view.hit_header`");
                }
            }
            let (served, hit) = observe_app(ctx).await?;
            let db = db_value(ctx).await?;
            let pass = served == db && (!self.need_hit || hit == Some(true));
            let source = match hit {
                Some(true) => " (cache hit)",
                Some(false) => " (cache miss)",
                None => "",
            };
            Ok(CheckResult {
                pass,
                detail: format!("served={served}{source} db={db}"),
            })
        })
    }
}

struct Compare(fn(&Option<String>, &str) -> bool);

impl Check for Compare {
    fn eval<'a>(&'a self, ctx: &'a mut RunCtx<'_>) -> BoxFut<'a, Result<CheckResult>> {
        Box::pin(async move {
            let (cache, db) = observe(ctx).await?;
            let pass = (self.0)(&cache, &db);
            Ok(CheckResult {
                pass,
                detail: format!("cache={} db={db}", cache.as_deref().unwrap_or("<absent>")),
            })
        })
    }
}
