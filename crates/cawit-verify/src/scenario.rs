//! Scenario file format (docs/design/verify-design.md 4).

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub name: String,
    #[serde(default = "one")]
    pub runs: u32,
    /// Named application base URLs, e.g. `a: http://127.0.0.1:3000`.
    pub apps: BTreeMap<String, String>,
    /// Redis used for the sync protocol and for reading cache state.
    pub redis: String,
    /// PostgreSQL connection string, for checks that compare with the database.
    #[serde(default)]
    pub postgres: Option<String>,
    #[serde(default)]
    pub requests: BTreeMap<String, RequestTemplate>,
    #[serde(default)]
    pub state: Option<StateSpec>,
    /// Each step is a one-key mapping: `{ <kind>: <params> }`.
    pub steps: Vec<serde_yaml::Value>,
}

fn one() -> u32 {
    1
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestTemplate {
    #[serde(default = "post")]
    pub method: String,
    pub path: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub json: Option<serde_json::Value>,
    /// Treat the response as failed if its JSON body has this top-level field
    /// (e.g. `errors` for GraphQL, which reports errors with HTTP 200).
    #[serde(default)]
    pub error_field: Option<String>,
}

fn post() -> String {
    "POST".into()
}

/// What the checks compare: one cache entry (optionally one JSON field of it)
/// and one database value (first column of the first row, cast to text in SQL).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateSpec {
    pub cache_key: String,
    #[serde(default)]
    pub cache_field: Option<String>,
    pub db_query: String,
}

impl Scenario {
    pub fn load(path: &Path) -> Result<Scenario> {
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Scenario::parse(&text).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Scenario> {
        Ok(serde_yaml::from_str(text)?)
    }
}
