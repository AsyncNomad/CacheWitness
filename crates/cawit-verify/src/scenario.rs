//! The scenario file format. The user-facing reference is
//! [scenario format reference](https://github.com/AsyncNomad/CacheWitness/blob/main/docs/book/en/src/reference/scenario-format.md).
//!
//! Every struct rejects unknown fields, so a typo in a scenario fails to load
//! instead of being ignored.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

/// A scenario: where the application and its stores are, which requests to
/// send, what to judge, and either `steps` or a `pattern`.
///
/// ```
/// use cawit_verify::Scenario;
///
/// let sc = Scenario::parse(r#"
/// name: demo
/// apps: { a: "http://127.0.0.1:3000", b: "http://127.0.0.1:3001" }
/// redis: redis://127.0.0.1:6379
/// requests:
///   update: { method: PATCH, path: /articles/1, json: { title: "${value}" } }
///   read: { method: GET, path: /articles/1 }
/// state:
///   db_query: "select title from articles where id = 1"
///   app_view: { app: a, request: read, field: /title }
/// pattern:
///   stale-set: { reader: a, writer: b, write: update, read: read }
/// "#).unwrap();
/// assert_eq!(sc.runs, 1);
/// assert!(sc.resolved_steps().unwrap().len() > 5);
/// ```
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    /// Name used in output and as the JUnit suite name.
    pub name: String,
    /// Number of runs (default 1); `--runs` overrides it.
    #[serde(default = "one")]
    pub runs: u32,
    /// Named application base URLs, e.g. `a: http://127.0.0.1:3000`
    /// (`${env.X}` allowed).
    pub apps: BTreeMap<String, String>,
    /// Redis used for the sync protocol and for reading cache state
    /// (`${env.X}` allowed, e.g. for a password).
    pub redis: String,
    /// PostgreSQL connection string, for checks that compare with the database
    /// (`${env.X}` allowed).
    #[serde(default)]
    pub postgres: Option<String>,
    /// Named request templates, referenced by steps and patterns.
    #[serde(default)]
    pub requests: BTreeMap<String, RequestTemplate>,
    /// What checks compare; required by the built-in checks.
    #[serde(default)]
    pub state: Option<StateSpec>,
    /// Each step is a one-key mapping: `{ <kind>: <params> }`.
    #[serde(default)]
    pub steps: Vec<serde_yaml::Value>,
    /// A built-in pattern, `{ <name>: <params> }`, instead of `steps`.
    #[serde(default)]
    pub pattern: Option<serde_yaml::Value>,
    /// TLS for `rediss://` and `https://` (a private CA, or no verification).
    #[serde(default)]
    pub tls: Option<TlsSpec>,
    /// Directory of the scenario file, for relative paths (set by [`Scenario::load`]).
    #[serde(skip)]
    pub base_dir: Option<std::path::PathBuf>,
}

/// TLS options for Redis (`rediss://`) and application servers (`https://`).
/// PostgreSQL follows `sslmode` in its connection string instead
/// ([`crate::pg_tls`]).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TlsSpec {
    /// Extra trusted CA certificates (PEM), relative to the scenario file.
    #[serde(default)]
    pub ca_file: Option<std::path::PathBuf>,
    /// Do not verify certificates (self-signed test servers only).
    #[serde(default)]
    pub insecure: bool,
}

impl Scenario {
    /// TLS options with `ca_file` resolved against the scenario's directory.
    pub fn tls_options(&self) -> cawit_resp::tls::TlsOptions {
        let spec = self.tls.clone().unwrap_or_default();
        let ca_file = spec.ca_file.map(|p| match &self.base_dir {
            Some(dir) if p.is_relative() => dir.join(p),
            _ => p,
        });
        cawit_resp::tls::TlsOptions {
            ca_file,
            insecure: spec.insecure,
        }
    }
}

fn one() -> u32 {
    1
}

/// An HTTP request template. `path`, header values, and every string in
/// `json` may use `${...}` variables ([`crate::template::render`]).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestTemplate {
    /// HTTP method (default `POST`).
    #[serde(default = "post")]
    pub method: String,
    /// Path and query, e.g. `/items/articles/1?fields=title`.
    pub path: String,
    /// Extra request headers.
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// JSON body, sent with `content-type: application/json`.
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
    /// Redis key of the cache entry (used as written, no variables).
    #[serde(default)]
    pub cache_key: Option<String>,
    /// Top-level field to take from the entry when it holds a JSON object.
    #[serde(default)]
    pub cache_field: Option<String>,
    /// SQL returning the committed value in the first column of the first
    /// row, as text (cast with `::text` if needed).
    pub db_query: String,
    /// Read the value through the application instead of the cache entry.
    #[serde(default)]
    pub app_view: Option<AppView>,
}

/// Read the value through the application: send `request` to `app` and take
/// the JSON pointer `field` (e.g. `/data/title`) of the response body.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppView {
    /// App to send the request to.
    pub app: String,
    /// Request template to send.
    pub request: String,
    /// JSON pointer to the value in the response body, e.g. `/data/title`.
    pub field: String,
    /// Response header that tells whether the cache served the response,
    /// and the value that means it did (default `HIT`).
    #[serde(default)]
    pub hit_header: Option<String>,
    /// Value of `hit_header` that means a cache hit.
    #[serde(default = "hit")]
    pub hit_value: String,
}

fn hit() -> String {
    "HIT".into()
}

impl Scenario {
    /// Read and parse a scenario file.
    pub fn load(path: &Path) -> Result<Scenario> {
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let mut sc = Scenario::parse(&text).with_context(|| format!("parsing {}", path.display()))?;
        sc.base_dir = path.parent().map(|d| d.to_path_buf());
        Ok(sc)
    }

    /// Parse a scenario from YAML. Exactly one of `steps` and `pattern` must
    /// be present.
    pub fn parse(text: &str) -> Result<Scenario> {
        let sc: Scenario = serde_yaml::from_str(text)?;
        match (sc.steps.is_empty(), sc.pattern.is_some()) {
            (true, false) => anyhow::bail!("a scenario needs `steps` or `pattern`"),
            (false, true) => anyhow::bail!("a scenario has either `steps` or `pattern`, not both"),
            _ => Ok(sc),
        }
    }

    /// The steps to run: `steps` as written, or the expansion of `pattern`
    /// by the built-in patterns.
    pub fn resolved_steps(&self) -> Result<Vec<serde_yaml::Value>> {
        self.resolved_steps_with(&crate::patterns::Registry::builtin())
    }

    /// Like [`Scenario::resolved_steps`], expanding `pattern` with `patterns`.
    pub fn resolved_steps_with(&self, patterns: &crate::patterns::Registry) -> Result<Vec<serde_yaml::Value>> {
        match &self.pattern {
            Some(p) => patterns.expand(self, p),
            None => Ok(self.steps.clone()),
        }
    }
}
