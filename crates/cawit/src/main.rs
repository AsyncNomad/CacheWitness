//! `cawit` command line.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use cawit_verify::{Outcome, RunResult, runner::Options};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "cawit", version, about = "Force cache races in a fixed order and prove the fix")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run a scenario and judge every run (exit 0 all pass, 1 any fail, 2 inconclusive).
    Verify {
        /// Scenario file (YAML).
        scenario: PathBuf,
        /// Number of runs (overrides `runs` in the scenario).
        #[arg(long)]
        runs: Option<u32>,
        /// Write the full report as JSON to this path.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Write a JUnit XML report (one test case per run) to this path.
        #[arg(long)]
        junit: Option<PathBuf>,
        /// Print the steps the scenario runs (a pattern expanded) and exit.
        #[arg(long)]
        show_steps: bool,
    },
}

fn print_run(r: &RunResult) {
    let label = match r.outcome {
        Outcome::Pass => "PASS",
        Outcome::Fail => "FAIL",
        Outcome::NotReached => "NOT_REACHED",
        Outcome::Error => "ERROR",
    };
    println!("run {:>3}  {label}", r.run);
    for e in &r.expectations {
        println!(
            "          {} {} ({}): {}",
            if e.pass { "ok  " } else { "FAIL" },
            e.name,
            e.check,
            e.detail
        );
    }
    if let Some(n) = &r.note {
        println!("          {n}");
    }
}

fn show_steps(scenario: PathBuf) -> Result<i32> {
    let sc = cawit_verify::Scenario::load(&scenario)?;
    print!("{}", serde_yaml::to_string(&sc.resolved_steps()?)?);
    Ok(0)
}

async fn verify(scenario: PathBuf, runs: Option<u32>, out: Option<PathBuf>, junit: Option<PathBuf>) -> Result<i32> {
    let sc = cawit_verify::Scenario::load(&scenario)?;
    let report = cawit_verify::run(&sc, &Options { runs }, print_run).await?;
    let mut per_check: Vec<(String, usize, usize)> = Vec::new();
    for r in report.runs.iter().filter(|r| matches!(r.outcome, Outcome::Pass | Outcome::Fail)) {
        for e in &r.expectations {
            match per_check.iter_mut().find(|(n, _, _)| *n == e.name) {
                Some(c) => {
                    c.1 += e.pass as usize;
                    c.2 += 1;
                }
                None => per_check.push((e.name.clone(), e.pass as usize, 1)),
            }
        }
    }
    println!();
    println!(
        "{}: {} runs  pass {}  fail {}  not_reached {}  error {}",
        report.scenario,
        report.runs.len(),
        report.count(Outcome::Pass),
        report.count(Outcome::Fail),
        report.count(Outcome::NotReached),
        report.count(Outcome::Error)
    );
    for (name, pass, total) in &per_check {
        println!("  {name:<20} passed {pass}/{total}");
    }
    if let Some(path) = out {
        std::fs::write(&path, serde_json::to_vec_pretty(&report)?).with_context(|| format!("writing {}", path.display()))?;
    }
    if let Some(path) = junit {
        std::fs::write(&path, report.junit()).with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(report.exit_code())
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Verify {
            scenario,
            show_steps: true,
            ..
        } => show_steps(scenario),
        Command::Verify {
            scenario,
            runs,
            out,
            junit,
            ..
        } => verify(scenario, runs, out, junit).await,
    };
    match result {
        Ok(code) => ExitCode::from(code as u8),
        Err(e) => {
            eprintln!("cawit: {e:#}");
            ExitCode::from(2)
        }
    }
}
