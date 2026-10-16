//! Every scenario shipped in the repository parses and its steps build.

use std::path::Path;

use cawit_verify::Scenario;
use cawit_verify::steps::Registry;

#[test]
fn shipped_scenarios_load() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lab/twenty/scenarios");
    let registry = Registry::builtin();
    let mut seen = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "yaml") {
            let sc = Scenario::load(&path).unwrap();
            for (i, step) in sc.steps.iter().enumerate() {
                registry.build(step).unwrap_or_else(|e| panic!("{} step {}: {e:#}", path.display(), i + 1));
            }
            seen += 1;
        }
    }
    assert!(seen > 0, "no scenarios in {}", dir.display());
}
