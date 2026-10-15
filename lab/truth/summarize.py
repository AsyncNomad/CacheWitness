"""Turn a matrix list (from matrix.sh) into a markdown table.

Usage: python3 -I lab/truth/summarize.py lab/out/matrix-<time>.txt
"""

import ast
import json
import re
import sys
from pathlib import Path

# What the prototype should answer for each true cause.
EXPECTED = {"READ_BEFORE_COMMIT": "STALE_SET_RACE", "READ_AFTER_COMMIT_TS": "STALE_SET_RACE",
            "REPLICA_READ": "STALE_READ_AFTER_INVALIDATION"}


def stats(run):
    line = (run / "stats.out").read_text().strip()
    m = re.match(r"stats (\[.*\])", line)
    return dict(ast.literal_eval(m.group(1))) if m else {}


def main(listfile):
    rows = []
    for line in Path(listfile).read_text().split("\n"):
        if not line.strip():
            continue
        mode, ratio, rep, run = line.split()
        run = Path(run)
        a = json.load(open(run / "analysis.json"))
        st = stats(run)
        truth = json.loads((run / "truth.out").read_text().strip().splitlines()[-1])
        wrong = [v for v in a["violations"] if v["truth_cause"] is not None and v["tool_cause"] != EXPECTED[v["truth_cause"]]]
        rows.append({
            "mode": mode, "ratio": ratio, "rep": rep,
            "writes": truth["updates"],
            "harmful_sets": a["stale_sets"]["by_order_vs_del"].get("AFTER_DEL", 0),
            "overlap_sets": a["stale_sets"]["by_order_vs_del"].get("OVERLAP", 0),
            "tool_stale_sets": st.get("stale_sets_seen", 0),
            "stale_reads": a["stale_reads_served_ms_after_commit"]["n"],
            "stale_reads_100": a["stale_reads_served_ms_after_commit"]["over_100ms"],
            "oracle": a["end_state"]["oracle_stale_keys"],
            "detected": a["end_state"]["oracle_detected"],
            "tool_errors": len(a["violations"]),
            "transient": len(a["end_state"]["tool_only"]),
            "wrong_cause": len(wrong),
            "wrong_detail": ", ".join(f"{v['key']}:{v['tool_cause']}<-{v['truth_cause']}" for v in wrong),
            "miss_diff": a["miss_attribution"].get("different request", 0),
            "old_after_cts": a["old_reads_after_newer_commit_ts"]["n"],
            "old_after_ack": a["old_reads_after_newer_commit_ts"]["after_writer_ack"],
            "ack_p50": a["commit_ts_to_writer_ack_ms"]["p50"],
            "rechecks": st.get("unknown_rechecks", 0),
        })
    cols = ["mode", "ratio", "rep", "writes", "harmful_sets", "overlap_sets", "tool_stale_sets", "stale_reads",
            "stale_reads_100", "oracle", "detected", "tool_errors", "transient", "wrong_cause", "miss_diff",
            "old_after_cts", "old_after_ack", "ack_p50", "rechecks"]
    print("| " + " | ".join(cols) + " |")
    print("|" + "---|" * len(cols))
    for r in rows:
        print("| " + " | ".join(str(r[c]) for c in cols) + " |")
    for r in rows:
        if r["wrong_detail"]:
            print(f"\nwrong cause in {r['mode']} {r['ratio']} #{r['rep']}: {r['wrong_detail']}")


if __name__ == "__main__":
    main(sys.argv[1])
