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
    #[serde(default)]
    pub steps: Vec<serde_yaml::Value>,
    /// A built-in pattern, `{ <name>: <params> }`, instead of `steps`.
    #[serde(default)]
    pub pattern: Option<serde_yaml::Value>,
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

/// What the checks compare with the database value (first column of the
/// first row of `db_query`, cast to text in SQL): either one cache entry
/// (optionally one JSON field of it), or what the application serves
/// (`app_view`), for caches whose keys are opaque or whose values are encoded.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateSpec {
    #[serde(default)]
    pub cache_key: Option<String>,
    #[serde(default)]
    pub cache_field: Option<String>,
    pub db_query: String,
    #[serde(default)]
    pub app_view: Option<AppView>,
}

/// Read the value through the application: send `request` to `app` and take
/// the JSON pointer `field` (e.g. `/data/title`) of the response body.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppView {
    pub app: String,
    pub request: String,
    pub field: String,
    /// Response header that tells whether the cache served the response,
    /// and the value that means it did (default `HIT`).
    #[serde(default)]
    pub hit_header: Option<String>,
    #[serde(default = "hit")]
    pub hit_value: String,
}

fn hit() -> String {
    "HIT".into()
}

impl Scenario {
    pub fn load(path: &Path) -> Result<Scenario> {
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Scenario::parse(&text).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Scenario> {
        let sc: Scenario = serde_yaml::from_str(text)?;
        match (sc.steps.is_empty(), sc.pattern.is_some()) {
            (true, false) => anyhow::bail!("a scenario needs `steps` or `pattern`"),
            (false, true) => anyhow::bail!("a scenario has either `steps` or `pattern`, not both"),
            _ => Ok(sc),
        }
    }

    /// The steps to run: `steps` as written, or the expansion of `pattern`.
    pub fn resolved_steps(&self) -> Result<Vec<serde_yaml::Value>> {
        match &self.pattern {
            Some(p) => crate::patterns::Registry::builtin().expand(self, p),
            None => Ok(self.steps.clone()),
        }
    }
}
