//! CacheWitness verify engine (docs/design/verify-design.md).
//!
//! A scenario is a list of steps run in order, several times. Steps are built
//! from YAML through a [`steps::Registry`] and verdicts come from a
//! [`checks::Registry`], so new step kinds and checks plug in without touching
//! the runner.

pub mod checks;
pub mod http;
pub mod patterns;
pub mod runner;
pub mod scenario;
pub mod steps;
pub mod template;

pub use runner::{Outcome, Report, RunResult, run};
pub use scenario::Scenario;
