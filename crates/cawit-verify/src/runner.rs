//! Runs a scenario several times and collects outcomes
//! (docs/design/verify-design.md 5).

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::checks;
use crate::scenario::Scenario;
use crate::steps::{self, Expectation, Flow, RunCtx, Step};
use crate::template::Vars;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Outcome {
    Pass,
    Fail,
    /// The forced order did not happen; the run is discarded.
    NotReached,
    /// A request or step failed; the run is discarded.
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunResult {
    pub run: u32,
    pub outcome: Outcome,
    pub expectations: Vec<Expectation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub scenario: String,
    pub runs: Vec<RunResult>,
}

impl Report {
    pub fn count(&self, o: Outcome) -> usize {
        self.runs.iter().filter(|r| r.outcome == o).count()
    }

    /// 0 all passed, 1 any failure, 2 no failure but some runs discarded.
    pub fn exit_code(&self) -> i32 {
        if self.count(Outcome::Fail) > 0 {
            1
        } else if self.count(Outcome::Pass) == self.runs.len() && !self.runs.is_empty() {
            0
        } else {
            2
        }
    }
}

pub struct Options {
    pub runs: Option<u32>,
}

pub async fn run(scenario: &Scenario, opts: &Options, mut on_run: impl FnMut(&RunResult)) -> Result<Report> {
    let registry = steps::Registry::builtin();
    let check_registry = checks::Registry::builtin();
    let steps: Vec<Box<dyn Step>> = scenario
        .steps
        .iter()
        .enumerate()
        .map(|(i, v)| registry.build(v).with_context(|| format!("step {}", i + 1)))
        .collect::<Result<_>>()?;

    let pg = match &scenario.postgres {
        Some(conn) => {
            let (client, connection) = tokio_postgres::connect(conn, tokio_postgres::NoTls).await.context("connecting to postgres")?;
            tokio::spawn(async move {
                let _ = connection.await;
            });
            Some(client)
        }
        None => None,
    };

    let run_id = format!("{:x}", SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis());
    let total = opts.runs.unwrap_or(scenario.runs);
    let mut results = Vec::new();
    for i in 1..=total {
        let redis = cawit_resp::Client::connect(&scenario.redis).await.with_context(|| format!("connecting to {}", scenario.redis))?;
        let mut vars = Vars::new();
        vars.insert("run".into(), format!("{run_id}-{i}"));
        vars.insert("run_index".into(), i.to_string());
        let mut ctx = RunCtx {
            scenario,
            checks: &check_registry,
            vars,
            redis,
            pg: pg.as_ref(),
            syncs: BTreeMap::new(),
            tasks: BTreeMap::new(),
            expectations: Vec::new(),
        };
        let mut outcome = None;
        let mut note = None;
        for (n, step) in steps.iter().enumerate() {
            match step.run(&mut ctx).await {
                Ok(Flow::Continue) => {}
                Ok(Flow::NotReached(why)) => {
                    outcome = Some(Outcome::NotReached);
                    note = Some(why);
                    break;
                }
                Err(e) => {
                    outcome = Some(Outcome::Error);
                    note = Some(format!("step {} ({}): {e:#}", n + 1, step.kind()));
                    break;
                }
            }
        }
        cleanup(&mut ctx).await;
        let outcome = outcome.unwrap_or(if ctx.expectations.iter().all(|e| e.pass) { Outcome::Pass } else { Outcome::Fail });
        let result = RunResult { run: i, outcome, expectations: ctx.expectations, note };
        on_run(&result);
        results.push(result);
    }
    Ok(Report { scenario: scenario.name.clone(), runs: results })
}

/// Never leave an app blocked or a sync point armed after a run.
async fn cleanup(ctx: &mut RunCtx<'_>) {
    let armed: Vec<_> = ctx.syncs.values().cloned().collect();
    for a in armed {
        let _ = ctx.redis.cmd(&["DEL", &format!("cawit:arm:{}", a.point)]).await;
        let _ = ctx.redis.cmd(&["RPUSH", &format!("cawit:release:{}", a.token), "cleanup"]).await;
        let _ = ctx.redis.cmd(&["EXPIRE", &format!("cawit:release:{}", a.token), "60"]).await;
        let _ = ctx.redis.cmd(&["DEL", &format!("cawit:reached:{}", a.token)]).await;
    }
    for (_, task) in std::mem::take(&mut ctx.tasks) {
        let _ = tokio::time::timeout(std::time::Duration::from_secs(10), task).await;
    }
}
