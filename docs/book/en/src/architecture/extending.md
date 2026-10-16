# Extending the Engine

Step kinds, checks, and patterns live in registries carried by `runner::Options`. Register your own and pass the options to `cawit_verify::run`. Full API: [`cawit-verify` on docs.rs](https://docs.rs/cawit-verify).

## A step kind

```rust,ignore
use cawit_verify::runner::Options;
use cawit_verify::steps::{BoxFut, Flow, RunCtx, Step};

/// `{ redis: { cmd: [DEL, "app:article:1"] } }`: send one Redis command.
#[derive(serde::Deserialize)]
struct RedisCmd {
    cmd: Vec<String>,
}

impl Step for RedisCmd {
    fn kind(&self) -> &'static str {
        "redis"
    }
    fn run<'a>(&'a self, ctx: &'a mut RunCtx<'_>) -> BoxFut<'a, anyhow::Result<Flow>> {
        Box::pin(async move {
            ctx.redis.cmd(&self.cmd).await?;
            Ok(Flow::Continue)
        })
    }
}

let mut opts = Options::default();
opts.steps.register("redis", |params| Ok(Box::new(serde_yaml::from_value::<RedisCmd>(params)?)));
```

Return `Err` to end the run as `ERROR`, or `Ok(Flow::NotReached(reason))` to discard it because the forced order did not happen.

## A check

```rust,ignore
use cawit_verify::checks::{Check, CheckResult, observe};
use cawit_verify::steps::{BoxFut, RunCtx};

/// Passes when the cache entry is absent or holds valid JSON.
struct CacheIsJson;

impl Check for CacheIsJson {
    fn eval<'a>(&'a self, ctx: &'a mut RunCtx<'_>) -> BoxFut<'a, anyhow::Result<CheckResult>> {
        Box::pin(async move {
            let (cache, _db) = observe(ctx).await?;
            let pass = cache.as_deref().is_none_or(|v| serde_json::from_str::<serde_json::Value>(v).is_ok());
            Ok(CheckResult { pass, detail: format!("cache={cache:?}") })
        })
    }
}

opts.checks.register("cache_is_json", Box::new(CacheIsJson));
```

`checks::observe` reads `state.cache_key` and `state.db_query`; `checks::observe_app` reads through `state.app_view`.

## A pattern

A pattern is a function from its YAML parameters to a list of steps. It receives the scenario, so it can pick checks from `state`.

```rust,ignore
fn read_twice(_sc: &Scenario, p: serde_yaml::Value) -> anyhow::Result<Vec<serde_yaml::Value>> {
    let app = p["app"].as_str().unwrap_or("a").to_string();
    let step: serde_yaml::Value = serde_yaml::from_str(&format!("{{ http: {{ app: {app}, request: read }} }}"))?;
    Ok(vec![step.clone(), step, serde_yaml::from_str("{ expect: { name: fresh, check: app_equals_db } }")?])
}

opts.patterns.register("read-twice", read_twice);
```

## Contributing a built-in

New built-in patterns are welcome when they come with a case study: a real application, a hook placement, and results showing which fixes pass. See [Contributing](../project/contributing.md).
