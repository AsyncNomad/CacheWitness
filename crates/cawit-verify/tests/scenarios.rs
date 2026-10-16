//! Every scenario shipped in the repository (lab/<target>/scenarios/*.yaml)
//! parses and its steps build.

use std::path::Path;

use cawit_verify::Scenario;
use cawit_verify::steps::Registry;

#[test]
fn shipped_scenarios_load() {
    let lab = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lab");
    let registry = Registry::builtin();
    let mut seen = Vec::new();
    for target in std::fs::read_dir(&lab).unwrap() {
        let dir = target.unwrap().path().join("scenarios");
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "yaml") {
                let sc = Scenario::load(&path).unwrap();
                for (i, step) in sc.resolved_steps().unwrap().iter().enumerate() {
                    registry.build(step).unwrap_or_else(|e| panic!("{} step {}: {e:#}", path.display(), i + 1));
                }
                seen.push(sc.name);
            }
        }
    }
    seen.sort();
    assert_eq!(seen, ["directus-failed-invalidation", "directus-failed-invalidation-bounded", "directus-invalidate-before-commit", "directus-stale-set", "twenty-stale-set"]);
}
