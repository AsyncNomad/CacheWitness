# 扩展引擎

步骤类型、检查和模式都位于 `runner::Options` 持有的注册表中。注册你自己的实现，再把选项传给 `cawit_verify::run`。完整 API：[docs.rs 上的 `cawit-verify`](https://docs.rs/cawit-verify)。

## 步骤类型

```rust,ignore
use cawit_verify::runner::Options;
use cawit_verify::steps::{BoxFut, Flow, RunCtx, Step};

/// `{ redis: { cmd: [DEL, "app:article:1"] } }`：发送一条 Redis 命令。
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

返回 `Err` 会让该轮以 `ERROR` 结束；返回 `Ok(Flow::NotReached(原因))` 则表示强制的顺序没有发生，该轮被丢弃。

## 检查

```rust,ignore
use cawit_verify::checks::{Check, CheckResult, observe};
use cawit_verify::steps::{BoxFut, RunCtx};

/// 缓存项不存在或是合法 JSON 时通过。
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

`checks::observe` 读取 `state.cache_key` 和 `state.db_query`；`checks::observe_app` 通过 `state.app_view` 读取。

## 模式

模式是一个把 YAML 参数转换为步骤列表的函数。它会收到场景本身，因此可以根据 `state` 选择检查。

```rust,ignore
fn read_twice(_sc: &Scenario, p: serde_yaml::Value) -> anyhow::Result<Vec<serde_yaml::Value>> {
    let app = p["app"].as_str().unwrap_or("a").to_string();
    let step: serde_yaml::Value = serde_yaml::from_str(&format!("{{ http: {{ app: {app}, request: read }} }}"))?;
    Ok(vec![step.clone(), step, serde_yaml::from_str("{ expect: { name: fresh, check: app_equals_db } }")?])
}

opts.patterns.register("read-twice", read_twice);
```

## 贡献内置模式

欢迎附带案例研究的新内置模式：一个真实应用、钩子位置，以及展示哪些修复能通过的结果。见[参与贡献](../project/contributing.md)。
