//! Scenario steps. Each step kind is built from YAML by a [`Registry`]
//! builder; add kinds with [`Registry::register`] without touching the runner.

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use cawit_resp::Frame;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use tokio::task::JoinHandle;

use crate::checks;
use crate::http;
use crate::scenario::Scenario;
use crate::template::{Vars, render, render_json};

pub type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What the runner does after a step.
#[derive(Debug, Clone, PartialEq)]
pub enum Flow {
    Continue,
    /// A sync point was not reached: the forced order did not happen and the
    /// run proves nothing (manipulation check).
    NotReached(String),
}

/// Result of one `expect` step.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Expectation {
    pub name: String,
    pub check: String,
    pub pass: bool,
    pub detail: String,
}

/// An armed sync point, remembered so the run can clean it up.
#[derive(Debug, Clone)]
pub struct Armed {
    pub point: String,
    pub token: String,
}

/// Per-run state shared by steps.
pub struct RunCtx<'s> {
    pub scenario: &'s Scenario,
    pub checks: &'s checks::Registry,
    pub vars: Vars,
    pub redis: cawit_resp::Client,
    pub pg: Option<&'s tokio_postgres::Client>,
    pub syncs: BTreeMap<String, Armed>,
    pub tasks: BTreeMap<String, JoinHandle<Result<http::Response>>>,
    pub expectations: Vec<Expectation>,
}

pub trait Step: Send + Sync {
    fn kind(&self) -> &'static str;
    fn run<'a>(&'a self, ctx: &'a mut RunCtx<'_>) -> BoxFut<'a, Result<Flow>>;
}

type Builder = fn(serde_yaml::Value) -> Result<Box<dyn Step>>;

pub struct Registry {
    builders: HashMap<&'static str, Builder>,
}

fn params<T: DeserializeOwned>(kind: &str, v: serde_yaml::Value) -> Result<T> {
    serde_yaml::from_value(v).with_context(|| format!("bad parameters for step `{kind}`"))
}

impl Registry {
    /// The built-in step kinds: http, arm, reached, release, await, sleep, expect.
    pub fn builtin() -> Registry {
        let mut r = Registry { builders: HashMap::new() };
        r.register("http", |v| Ok(Box::new(params::<HttpStep>("http", v)?)));
        r.register("arm", |v| Ok(Box::new(params::<ArmStep>("arm", v)?)));
        r.register("reached", |v| Ok(Box::new(params::<ReachedStep>("reached", v)?)));
        r.register("release", |v| Ok(Box::new(params::<ReleaseStep>("release", v)?)));
        r.register("await", |v| Ok(Box::new(params::<AwaitStep>("await", v)?)));
        r.register("sleep", |v| Ok(Box::new(params::<SleepStep>("sleep", v)?)));
        r.register("expect", |v| Ok(Box::new(params::<ExpectStep>("expect", v)?)));
        r
    }

    pub fn register(&mut self, kind: &'static str, builder: Builder) {
        self.builders.insert(kind, builder);
    }

    /// Build one step from `{ <kind>: <params> }`.
    pub fn build(&self, v: &serde_yaml::Value) -> Result<Box<dyn Step>> {
        let map = v.as_mapping().context("a step must be a mapping like `{ http: {...} }`")?;
        if map.len() != 1 {
            bail!("a step must have exactly one kind, got {}", map.len());
        }
        let (k, p) = map.iter().next().unwrap();
        let kind = k.as_str().context("step kind must be a string")?;
        let builder = self.builders.get(kind).ok_or_else(|| anyhow!("unknown step kind `{kind}`"))?;
        builder(p.clone())
    }
}

// ---------------------------------------------------------------- http

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HttpStep {
    app: String,
    request: String,
    #[serde(default)]
    vars: BTreeMap<String, String>,
    /// Run in the background under this name; `await` collects it.
    #[serde(default, rename = "async")]
    background: Option<String>,
    /// The request is expected to fail (e.g. under an injected fault): an
    /// error response does not end the run.
    #[serde(default)]
    allow_error: bool,
}

/// A rendered request, ready to send (owned, so it can run in a background task).
pub struct Prepared {
    base: String,
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Option<Vec<u8>>,
    error_field: Option<String>,
}

/// Render the named request template for `app` with the run variables plus `extra`.
pub fn prepare(ctx: &RunCtx<'_>, app: &str, request: &str, extra: &BTreeMap<String, String>) -> Result<Prepared> {
    let base = ctx.scenario.apps.get(app).with_context(|| format!("unknown app `{app}`"))?.clone();
    let tpl = ctx
        .scenario
        .requests
        .get(request)
        .with_context(|| format!("unknown request `{request}`"))?;
    let mut vars = ctx.vars.clone();
    for (k, v) in extra {
        vars.insert(k.clone(), render(v, &ctx.vars)?);
    }
    let path = render(&tpl.path, &vars)?;
    let mut headers: Vec<(String, String)> = tpl
        .headers
        .iter()
        .map(|(k, v)| Ok((k.clone(), render(v, &vars)?)))
        .collect::<Result<_>>()?;
    let body = match &tpl.json {
        Some(j) => {
            headers.push(("content-type".into(), "application/json".into()));
            Some(serde_json::to_vec(&render_json(j, &vars)?)?)
        }
        None => None,
    };
    Ok(Prepared {
        base,
        method: tpl.method.clone(),
        path,
        headers,
        body,
        error_field: tpl.error_field.clone(),
    })
}

impl Prepared {
    /// Send it; HTTP errors and `error_field` in the body are errors.
    pub async fn send(self) -> Result<http::Response> {
        let Prepared {
            base,
            method,
            path,
            headers,
            body,
            error_field,
        } = self;
        let resp = http::request(&base, &method, &path, &headers, body).await?;
        let text = || String::from_utf8_lossy(&resp.body).chars().take(300).collect::<String>();
        if resp.status >= 400 {
            bail!("HTTP {} from {base}{path}: {}", resp.status, text());
        }
        if let Some(field) = &error_field {
            let v: serde_json::Value = serde_json::from_slice(&resp.body).unwrap_or_default();
            if v.get(field).is_some_and(|e| !e.is_null()) {
                bail!("`{field}` in response from {base}{path}: {}", text());
            }
        }
        Ok(resp)
    }
}

impl Step for HttpStep {
    fn kind(&self) -> &'static str {
        "http"
    }
    fn run<'a>(&'a self, ctx: &'a mut RunCtx<'_>) -> BoxFut<'a, Result<Flow>> {
        Box::pin(async move {
            let send = prepare(ctx, &self.app, &self.request, &self.vars)?.send();
            let allow_error = self.allow_error;
            let fut = async move {
                match send.await {
                    Err(e) if allow_error => Ok(http::Response {
                        status: 0,
                        headers: Vec::new(),
                        body: format!("{e:#}").into_bytes(),
                    }),
                    other => other,
                }
            };
            match &self.background {
                Some(name) => {
                    ctx.tasks.insert(name.clone(), tokio::spawn(fut));
                }
                None => {
                    fut.await?;
                }
            }
            Ok(Flow::Continue)
        })
    }
}

// ---------------------------------------------------------------- sync protocol (verify-design.md 3)

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArmStep {
    point: String,
    /// Only take a call for this key (default: any key).
    #[serde(default)]
    key: Option<String>,
    /// What the taken call does: `hold` until released, or `fail` (the hook
    /// throws, as if the operation at that point failed).
    #[serde(default)]
    action: ArmAction,
    #[serde(rename = "as")]
    alias: String,
}

#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ArmAction {
    #[default]
    Hold,
    Fail,
}

impl Step for ArmStep {
    fn kind(&self) -> &'static str {
        "arm"
    }
    fn run<'a>(&'a self, ctx: &'a mut RunCtx<'_>) -> BoxFut<'a, Result<Flow>> {
        Box::pin(async move {
            let token = format!("{}-{}", ctx.vars.get("run").cloned().unwrap_or_default(), self.alias);
            let key = match &self.key {
                Some(k) => render(k, &ctx.vars)?,
                None => "*".into(),
            };
            let reply = ctx
                .redis
                .cmd(&[
                    "SET",
                    &format!("cawit:arm:{}", self.point),
                    &arm_value(&token, self.action, &key),
                    "PX",
                    "30000",
                ])
                .await?;
            if let Frame::Error(e) = reply {
                bail!("arming {}: {e}", self.point);
            }
            ctx.syncs.insert(
                self.alias.clone(),
                Armed {
                    point: self.point.clone(),
                    token,
                },
            );
            Ok(Flow::Continue)
        })
    }
}

/// Protocol v2 arm value: `<token>|<action>|<key or *>`. The key comes last
/// so it may contain `|`.
fn arm_value(token: &str, action: ArmAction, key: &str) -> String {
    let action = match action {
        ArmAction::Hold => "hold",
        ArmAction::Fail => "fail",
    };
    format!("{token}|{action}|{key}")
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReachedStep {
    sync: String,
    #[serde(default = "ten_seconds")]
    timeout_ms: u64,
}

fn ten_seconds() -> u64 {
    10_000
}

impl Step for ReachedStep {
    fn kind(&self) -> &'static str {
        "reached"
    }
    fn run<'a>(&'a self, ctx: &'a mut RunCtx<'_>) -> BoxFut<'a, Result<Flow>> {
        Box::pin(async move {
            let armed = ctx
                .syncs
                .get(&self.sync)
                .with_context(|| format!("`{}` was never armed", self.sync))?
                .clone();
            let secs = format!("{:.3}", self.timeout_ms as f64 / 1000.0);
            let reply = ctx.redis.cmd(&["BLPOP", &format!("cawit:reached:{}", armed.token), &secs]).await?;
            if reply.is_nil() {
                return Ok(Flow::NotReached(format!(
                    "sync point `{}` not reached within {}ms",
                    armed.point, self.timeout_ms
                )));
            }
            Ok(Flow::Continue)
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReleaseStep {
    sync: String,
}

impl Step for ReleaseStep {
    fn kind(&self) -> &'static str {
        "release"
    }
    fn run<'a>(&'a self, ctx: &'a mut RunCtx<'_>) -> BoxFut<'a, Result<Flow>> {
        Box::pin(async move {
            let armed = ctx
                .syncs
                .get(&self.sync)
                .with_context(|| format!("`{}` was never armed", self.sync))?
                .clone();
            // A hold that already timed out let the call go on its own: the
            // order this release was meant to create did not happen.
            let timed_out = ctx.redis.cmd(&["LLEN", &format!("cawit:timeout:{}", armed.token)]).await?;
            if !matches!(timed_out, Frame::Integer(0)) {
                return Ok(Flow::NotReached(format!(
                    "the hold at `{}` timed out before release (hook CAWIT_SYNC_TIMEOUT_S); the forced order did not happen",
                    armed.point
                )));
            }
            ctx.redis.cmd(&["RPUSH", &format!("cawit:release:{}", armed.token), "go"]).await?;
            Ok(Flow::Continue)
        })
    }
}

// ---------------------------------------------------------------- control

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AwaitStep {
    task: String,
}

impl Step for AwaitStep {
    fn kind(&self) -> &'static str {
        "await"
    }
    fn run<'a>(&'a self, ctx: &'a mut RunCtx<'_>) -> BoxFut<'a, Result<Flow>> {
        Box::pin(async move {
            let handle = ctx
                .tasks
                .remove(&self.task)
                .with_context(|| format!("no background task `{}`", self.task))?;
            handle.await.map_err(|e| anyhow!("task `{}` panicked: {e}", self.task))??;
            Ok(Flow::Continue)
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SleepStep {
    ms: u64,
}

impl Step for SleepStep {
    fn kind(&self) -> &'static str {
        "sleep"
    }
    fn run<'a>(&'a self, _ctx: &'a mut RunCtx<'_>) -> BoxFut<'a, Result<Flow>> {
        Box::pin(async move {
            tokio::time::sleep(Duration::from_millis(self.ms)).await;
            Ok(Flow::Continue)
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpectStep {
    name: String,
    check: String,
    /// Bounded-staleness verdict: pass if the check passes at some point
    /// within this many ms (polled every 50 ms). Without it the verdict is
    /// strict: the check must pass the first time it is evaluated.
    #[serde(default)]
    within_ms: Option<u64>,
}

impl Step for ExpectStep {
    fn kind(&self) -> &'static str {
        "expect"
    }
    fn run<'a>(&'a self, ctx: &'a mut RunCtx<'_>) -> BoxFut<'a, Result<Flow>> {
        Box::pin(async move {
            let check = ctx
                .checks
                .get(&self.check)
                .with_context(|| format!("unknown check `{}`", self.check))?;
            let start = tokio::time::Instant::now();
            let mut r = check.eval(ctx).await?;
            if let Some(limit) = self.within_ms {
                let limit = Duration::from_millis(limit);
                while !r.pass && start.elapsed() < limit {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    r = check.eval(ctx).await?;
                }
                r.detail = format!(
                    "{} (after {}ms, limit {}ms)",
                    r.detail,
                    start.elapsed().as_millis(),
                    limit.as_millis()
                );
            }
            ctx.expectations.push(Expectation {
                name: self.name.clone(),
                check: self.check.clone(),
                pass: r.pass,
                detail: r.detail,
            });
            Ok(Flow::Continue)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_known_steps_and_rejects_unknown() {
        let r = Registry::builtin();
        let ok: serde_yaml::Value = serde_yaml::from_str("{ arm: { point: p, as: r1 } }").unwrap();
        assert_eq!(r.build(&ok).unwrap().kind(), "arm");
        let bad: serde_yaml::Value = serde_yaml::from_str("{ teleport: {} }").unwrap();
        assert!(r.build(&bad).is_err());
        let two: serde_yaml::Value = serde_yaml::from_str("{ sleep: { ms: 1 }, await: { task: t } }").unwrap();
        assert!(r.build(&two).is_err());
        let typo: serde_yaml::Value = serde_yaml::from_str("{ sleep: { msec: 1 } }").unwrap();
        assert!(r.build(&typo).is_err());
        let fail: serde_yaml::Value = serde_yaml::from_str("{ arm: { point: p, as: f, action: fail } }").unwrap();
        assert!(r.build(&fail).is_ok());
        let bad_action: serde_yaml::Value = serde_yaml::from_str("{ arm: { point: p, as: f, action: explode } }").unwrap();
        assert!(r.build(&bad_action).is_err());
    }

    #[test]
    fn arm_value_puts_key_last() {
        assert_eq!(arm_value("t1", ArmAction::Hold, "*"), "t1|hold|*");
        assert_eq!(arm_value("t1", ArmAction::Fail, "a|b"), "t1|fail|a|b");
    }
}
