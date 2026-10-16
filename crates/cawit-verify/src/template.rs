//! `${name}` substitution in request templates. `${env.X}` reads the
//! environment; other names come from run and step variables.

use std::collections::BTreeMap;

use anyhow::{Result, bail};
use serde_json::Value;

pub type Vars = BTreeMap<String, String>;

pub fn render(s: &str, vars: &Vars) -> Result<String> {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            bail!("unclosed ${{ in {s:?}")
        };
        let name = &after[..end];
        let value = match name.strip_prefix("env.") {
            Some(env) => std::env::var(env).map_err(|_| anyhow::anyhow!("environment variable {env} is not set"))?,
            None => match vars.get(name) {
                Some(v) => v.clone(),
                None => bail!("unknown variable ${{{name}}}"),
            },
        };
        out.push_str(&value);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Render every string inside a JSON value.
pub fn render_json(v: &Value, vars: &Vars) -> Result<Value> {
    Ok(match v {
        Value::String(s) => Value::String(render(s, vars)?),
        Value::Array(items) => Value::Array(items.iter().map(|i| render_json(i, vars)).collect::<Result<_>>()?),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| Ok((k.clone(), render_json(v, vars)?)))
                .collect::<Result<_>>()?,
        ),
        other => other.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_vars_and_env() {
        let mut vars = Vars::new();
        vars.insert("run".into(), "r1".into());
        // SAFETY: test-only, single-threaded use of the environment.
        unsafe { std::env::set_var("CAWIT_TEST_TOKEN", "tok") };
        assert_eq!(render("${run}-a ${env.CAWIT_TEST_TOKEN}", &vars).unwrap(), "r1-a tok");
        assert!(render("${missing}", &vars).is_err());
        assert!(render("${run", &vars).is_err());
        let j = serde_json::json!({ "v": ["${run}", 1] });
        assert_eq!(render_json(&j, &vars).unwrap(), serde_json::json!({ "v": ["r1", 1] }));
    }
}
