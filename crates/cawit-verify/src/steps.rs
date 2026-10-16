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
}

impl Step for HttpStep {
    fn kind(&self) -> &'static str {
        "http"
    }
    fn run<'a>(&'a self, ctx: &'a mut RunCtx<'_>) -> BoxFut<'a, Result<Flow>> {
        Box::pin(async move {
            let base = ctx.scenario.apps.get(&self.app).with_context(|| format!("unknown app `{}`", self.app))?.clone();
            let tpl = ctx.scenario.requests.get(&self.request).with_context(|| format!("unknown request `{}`", self.request))?;
            let mut vars = ctx.vars.clone();
            for (k, v) in &self.vars {
                vars.insert(k.clone(), render(v, &ctx.vars)?);
            }
            let path = render(&tpl.path, &vars)?;
            let mut headers: Vec<(String, String)> =
                tpl.headers.iter().map(|(k, v)| Ok((k.clone(), render(v, &vars)?))).collect::<Result<_>>()?;
            let body = match &tpl.json {
                Some(j) => {
                    headers.push(("content-type".into(), "application/json".into()));
                    Some(serde_json::to_vec(&render_json(j, &vars)?)?)
                }
                None => None,
            };
            let method = tpl.method.clone();
            let error_field = tpl.error_field.clone();
            let fut = async move {
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
    /// Only hold a call for this key (default: any key).
    #[serde(default)]
    key: Option<String>,
    #[serde(rename = "as")]
    alias: String,
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
                .cmd(&["SET", &format!("cawit:arm:{}", self.point), &format!("{token}|{key}"), "PX", "30000"])
                .await?;
            if let Frame::Error(e) = reply {
                bail!("arming {}: {e}", self.point);
            }
            ctx.syncs.insert(self.alias.clone(), Armed { point: self.point.clone(), token });
            Ok(Flow::Continue)
        })
    }
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
            let armed = ctx.syncs.get(&self.sync).with_context(|| format!("`{}` was never armed", self.sync))?.clone();
            let secs = format!("{:.3}", self.timeout_ms as f64 / 1000.0);
            let reply = ctx.redis.cmd(&["BLPOP", &format!("cawit:reached:{}", armed.token), &secs]).await?;
            if reply.is_nil() {
                return Ok(Flow::NotReached(format!("sync point `{}` not reached within {}ms", armed.point, self.timeout_ms)));
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
            let armed = ctx.syncs.get(&self.sync).with_context(|| format!("`{}` was never armed", self.sync))?.clone();
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
            let handle = ctx.tasks.remove(&self.task).with_context(|| format!("no background task `{}`", self.task))?;
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
}

impl Step for ExpectStep {
    fn kind(&self) -> &'static str {
        "expect"
    }
    fn run<'a>(&'a self, ctx: &'a mut RunCtx<'_>) -> BoxFut<'a, Result<Flow>> {
        Box::pin(async move {
            let check = ctx.checks.get(&self.check).with_context(|| format!("unknown check `{}`", self.check))?;
            let r = check.eval(ctx).await?;
            ctx.expectations.push(Expectation { name: self.name.clone(), check: self.check.clone(), pass: r.pass, detail: r.detail });
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
    }
}
