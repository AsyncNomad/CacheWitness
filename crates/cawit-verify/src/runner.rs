//! Running a scenario: [`run`], its [`Options`], and the resulting [`Report`].
//!
//! | Outcome | When |
//! |---|---|
//! | [`Outcome::Pass`] | every `expect` passed |
//! | [`Outcome::Fail`] | an `expect` failed |
//! | [`Outcome::NotReached`] | a sync point was not reached in time, or a hold timed out before release: the forced order did not happen, so the run is discarded |
//! | [`Outcome::Error`] | a request or step failed; the run is discarded |
//!
//! [`Report::exit_code`] maps these to `0` (all passed), `1` (any failed),
//! and `2` (no failure, but runs were discarded).

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::checks;
use crate::patterns;
use crate::scenario::Scenario;
use crate::steps::{self, Expectation, Flow, RunCtx, Step};
use crate::template::Vars;

/// How a run ended. Serialized as `PASS`, `FAIL`, `NOT_REACHED`, `ERROR`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Outcome {
    /// Every expectation passed.
    Pass,
    /// At least one expectation failed.
    Fail,
    /// The forced order did not happen; the run is discarded.
    NotReached,
    /// A request or step failed; the run is discarded.
    Error,
}

/// The result of one run.
#[derive(Debug, Clone, Serialize)]
pub struct RunResult {
    /// Run number, from 1.
    pub run: u32,
    /// How the run ended.
    pub outcome: Outcome,
    /// The `expect` steps evaluated before the run ended, in order.
    pub expectations: Vec<Expectation>,
    /// Why the run ended early (`NotReached`, `Error`); `None` otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Results of all runs of a scenario. Serializes to the JSON written by
/// `cawit verify --out`.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    /// The scenario's `name`.
    pub scenario: String,
    /// One entry per run, in order.
    pub runs: Vec<RunResult>,
}

impl Report {
    /// Number of runs with outcome `o`.
    pub fn count(&self, o: Outcome) -> usize {
        self.runs.iter().filter(|r| r.outcome == o).count()
    }

    /// Process exit code: `0` all runs passed, `1` any run failed, `2` no run
    /// failed but some were discarded (or there were no runs).
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

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

impl Report {
    /// JUnit XML: one test case per run. Failed runs are failures; runs that
    /// did not reach a sync point or errored are reported as skipped, since
    /// they neither pass nor fail the fix.
    pub fn junit(&self) -> String {
        let name = xml_escape(&self.scenario);
        let skipped = self.count(Outcome::NotReached) + self.count(Outcome::Error);
        let mut x = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuite name=\"{name}\" tests=\"{}\" failures=\"{}\" skipped=\"{skipped}\">\n",
            self.runs.len(),
            self.count(Outcome::Fail)
        );
        for r in &self.runs {
            x += &format!("  <testcase classname=\"{name}\" name=\"run {}\">\n", r.run);
            let detail: Vec<String> = r
                .expectations
                .iter()
                .map(|e| format!("{} {} ({}): {}", if e.pass { "ok" } else { "FAIL" }, e.name, e.check, e.detail))
                .chain(r.note.clone())
                .collect();
            let detail = xml_escape(&detail.join("\n"));
            match r.outcome {
                Outcome::Pass => {}
                Outcome::Fail => {
                    let failed: Vec<&str> = r.expectations.iter().filter(|e| !e.pass).map(|e| e.name.as_str()).collect();
                    x += &format!("    <failure message=\"{}\">{detail}</failure>\n", xml_escape(&failed.join(", ")));
                }
                Outcome::NotReached | Outcome::Error => {
                    x += &format!("    <skipped message=\"{:?}\">{detail}</skipped>\n", r.outcome);
                }
            }
            x += "  </testcase>\n";
        }
        x + "</testsuite>\n"
    }
}

/// How to run a scenario: the number of runs and the registries that turn
/// YAML into steps, checks, and pattern expansions.
///
/// [`Options::default`] uses the built-in registries. Register your own step
/// kinds, checks, or patterns on these registries before calling [`run`].
pub struct Options {
    /// Number of runs; `None` uses the scenario's `runs`.
    pub runs: Option<u32>,
    /// Step kinds available to the scenario.
    pub steps: steps::Registry,
    /// Checks available to `expect` steps.
    pub checks: checks::Registry,
    /// Patterns available to `pattern:`.
    pub patterns: patterns::Registry,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            runs: None,
            steps: steps::Registry::builtin(),
            checks: checks::Registry::builtin(),
            patterns: patterns::Registry::builtin(),
        }
    }
}

/// Run `scenario` and collect a [`Report`].
///
/// Steps are built once, before the first run, so a scenario error (unknown
/// step kind, bad parameters, unknown pattern) fails fast with `Err`. Each run
/// then gets a fresh Redis connection and run variables (`run`, `run_index`),
/// executes the steps in order, and ends with cleanup: armed sync points are
/// disarmed, held calls are released, and background requests are awaited (up
/// to 10 s). `on_run` is called after each run, for progress output.
///
/// Failures inside a run become an [`Outcome`] (`Error`, `NotReached`), not an
/// `Err`. `Err` is returned only when the scenario cannot run at all: it does
/// not build, or PostgreSQL or Redis cannot be reached.
pub async fn run(scenario: &Scenario, opts: &Options, mut on_run: impl FnMut(&RunResult)) -> Result<Report> {
    let steps: Vec<Box<dyn Step>> = scenario
        .resolved_steps_with(&opts.patterns)?
        .iter()
        .enumerate()
        .map(|(i, v)| opts.steps.build(v).with_context(|| format!("step {}", i + 1)))
        .collect::<Result<_>>()?;

    let pg = match &scenario.postgres {
        Some(conn) => {
            let (client, connection) = tokio_postgres::connect(conn, tokio_postgres::NoTls)
                .await
                .context("connecting to postgres")?;
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
        let redis = cawit_resp::Client::connect(&scenario.redis)
            .await
            .with_context(|| format!("connecting to {}", scenario.redis))?;
        let mut vars = Vars::new();
        vars.insert("run".into(), format!("{run_id}-{i}"));
        vars.insert("run_index".into(), i.to_string());
        let mut ctx = RunCtx {
            scenario,
            checks: &opts.checks,
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
        let outcome = outcome.unwrap_or(if ctx.expectations.iter().all(|e| e.pass) {
            Outcome::Pass
        } else {
            Outcome::Fail
        });
        let result = RunResult {
            run: i,
            outcome,
            expectations: ctx.expectations,
            note,
        };
        on_run(&result);
        results.push(result);
    }
    Ok(Report {
        scenario: scenario.name.clone(),
        runs: results,
    })
}

/// Never leave an app blocked or a sync point armed after a run.
async fn cleanup(ctx: &mut RunCtx<'_>) {
    let armed: Vec<_> = ctx.syncs.values().cloned().collect();
    for a in armed {
        let _ = ctx.redis.cmd(&["DEL", &format!("cawit:arm:{}", a.point)]).await;
        let _ = ctx.redis.cmd(&["RPUSH", &format!("cawit:release:{}", a.token), "cleanup"]).await;
        let _ = ctx.redis.cmd(&["EXPIRE", &format!("cawit:release:{}", a.token), "60"]).await;
        let _ = ctx.redis.cmd(&["DEL", &format!("cawit:reached:{}", a.token)]).await;
        let _ = ctx.redis.cmd(&["DEL", &format!("cawit:timeout:{}", a.token)]).await;
    }
    for (_, task) in std::mem::take(&mut ctx.tasks) {
        let _ = tokio::time::timeout(std::time::Duration::from_secs(10), task).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(i: u32, outcome: Outcome) -> RunResult {
        let pass = outcome != Outcome::Fail;
        RunResult {
            run: i,
            outcome,
            expectations: vec![Expectation {
                name: "not_stale".into(),
                check: "cache_not_stale".into(),
                pass,
                detail: "cache=<a> db=b".into(),
            }],
            note: None,
        }
    }

    #[test]
    fn exit_codes() {
        let r = |o: Vec<Outcome>| Report {
            scenario: "s".into(),
            runs: o.into_iter().enumerate().map(|(i, o)| run(i as u32, o)).collect(),
        };
        assert_eq!(r(vec![Outcome::Pass, Outcome::Pass]).exit_code(), 0);
        assert_eq!(r(vec![Outcome::Pass, Outcome::Fail, Outcome::NotReached]).exit_code(), 1);
        assert_eq!(r(vec![Outcome::Pass, Outcome::NotReached]).exit_code(), 2);
        assert_eq!(r(vec![]).exit_code(), 2);
    }

    #[test]
    fn junit_marks_failures_and_skips_and_escapes() {
        let rep = Report {
            scenario: "s".into(),
            runs: vec![run(1, Outcome::Pass), run(2, Outcome::Fail), run(3, Outcome::NotReached)],
        };
        let x = rep.junit();
        assert!(x.contains(r#"tests="3" failures="1" skipped="1""#));
        assert!(x.contains(r#"<failure message="not_stale">"#));
        assert!(x.contains("cache=&lt;a&gt;"));
        assert!(x.contains("<skipped"));
    }
}
