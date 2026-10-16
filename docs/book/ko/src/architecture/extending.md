# 엔진 확장

단계 종류, 판정, 패턴은 `runner::Options`가 들고 있는 레지스트리에 있습니다. 직접 만든 것을 등록하고 그 옵션을 `cawit_verify::run`에 넘기십시오. 전체 API: [docs.rs의 `cawit-verify`](https://docs.rs/cawit-verify).

## 단계 종류

```rust,ignore
use cawit_verify::runner::Options;
use cawit_verify::steps::{BoxFut, Flow, RunCtx, Step};

/// `{ redis: { cmd: [DEL, "app:article:1"] } }`: Redis 명령 하나를 보냄.
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

`Err`를 돌려주면 회차가 `ERROR`로 끝나고, `Ok(Flow::NotReached(이유))`를 돌려주면 강제한 순서가 일어나지 않았으므로 회차를 버립니다.

## 판정

```rust,ignore
use cawit_verify::checks::{Check, CheckResult, observe};
use cawit_verify::steps::{BoxFut, RunCtx};

/// 캐시 항목이 없거나 올바른 JSON이면 통과.
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

`checks::observe`는 `state.cache_key`와 `state.db_query`를 읽고, `checks::observe_app`은 `state.app_view`를 거쳐 읽습니다.

## 패턴

패턴은 YAML 매개변수를 단계 목록으로 바꾸는 함수입니다. 시나리오를 함께 받으므로 `state`를 보고 판정을 고를 수 있습니다.

```rust,ignore
fn read_twice(_sc: &Scenario, p: serde_yaml::Value) -> anyhow::Result<Vec<serde_yaml::Value>> {
    let app = p["app"].as_str().unwrap_or("a").to_string();
    let step: serde_yaml::Value = serde_yaml::from_str(&format!("{{ http: {{ app: {app}, request: read }} }}"))?;
    Ok(vec![step.clone(), step, serde_yaml::from_str("{ expect: { name: fresh, check: app_equals_db } }")?])
}

opts.patterns.register("read-twice", read_twice);
```

## 내장 패턴 기여

새 내장 패턴은 사례 연구와 함께라면 환영합니다. 실제 애플리케이션, 훅 위치, 어떤 수정이 통과하는지 보여 주는 결과가 필요합니다. [기여하기](../project/contributing.md)를 보십시오.
