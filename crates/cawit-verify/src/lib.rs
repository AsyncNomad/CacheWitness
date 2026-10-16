//! The verify engine of [CacheWitness](https://github.com/AsyncNomad/CacheWitness):
//! force a cache race in the same order on every run, and judge each run
//! against the database.
//!
//! Most users want the `cawit` command line. Use this crate to embed the
//! runner in your own tooling, or to add step kinds, checks, and patterns.
//!
//! # How a scenario runs
//!
//! 1. A [`Scenario`] is loaded from YAML ([`Scenario::load`]). It names the
//!    application servers, Redis, PostgreSQL, request templates, the state to
//!    judge, and either hand-written `steps` or a built-in `pattern`.
//! 2. A pattern expands into ordinary steps ([`patterns::Registry`]); steps are
//!    built from YAML by a [`steps::Registry`].
//! 3. [`run`] executes the steps once per run. Steps send HTTP requests, arm
//!    *sync points* that the application reaches through a hook
//!    (`cawit-hook` for Node), wait for a held call, release it, and evaluate
//!    checks ([`checks::Registry`]).
//! 4. Each run ends as an [`Outcome`]; the [`Report`] gives an exit code and a
//!    JUnit rendering.
//!
//! The hook and the runner talk through Redis lists (the *hook protocol*,
//! described in the [hook protocol reference](https://github.com/AsyncNomad/CacheWitness/blob/main/docs/book/en/src/reference/hook-protocol.md)),
//! so the application can be in any language and on any host that reaches
//! the same Redis.
//!
//! # Example
//!
//! ```no_run
//! use cawit_verify::{Scenario, Outcome, runner::Options};
//!
//! # async fn demo() -> anyhow::Result<()> {
//! let scenario = Scenario::load("stale-set.yaml".as_ref())?;
//! let report = cawit_verify::run(&scenario, &Options::default(), |r| {
//!     println!("run {}: {:?}", r.run, r.outcome);
//! })
//! .await?;
//! assert_eq!(report.count(Outcome::Fail), 0);
//! std::process::exit(report.exit_code());
//! # }
//! ```
//!
//! # Extending
//!
//! Step kinds, checks, and patterns live in registries carried by
//! [`runner::Options`]; register your own before calling [`run`]. See
//! [`steps::Step`], [`checks::Check`], and [`patterns::Registry::register`].
//!
//! # Stability
//!
//! The API may change in any 0.x release. The scenario file format and the
//! hook protocol are the compatibility surface users depend on; changes to
//! them are called out in the changelog.

#![warn(missing_docs)]

pub mod checks;
pub mod http;
pub mod patterns;
pub mod runner;
pub mod scenario;
pub mod steps;
pub mod template;

pub use runner::{Outcome, Report, RunResult, run};
pub use scenario::Scenario;
