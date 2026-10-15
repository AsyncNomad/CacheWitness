"""Compare the prototype's verdicts with the ground truth recorded by run.js.

Usage: python3 -I lab/truth/analyze.py <run dir>

Reads truth.jsonl, commit_ts.json, end_state.json (from run.js) and
violations.jsonl (from the prototype). Prints a summary and writes
analysis.json next to the inputs.

All times are wall-clock milliseconds on the shared host clock.
"""

import bisect
import collections
import json
import re
import statistics
import sys
from pathlib import Path


def pct(xs, p):
    if not xs:
        return None
    xs = sorted(xs)
    return round(xs[min(len(xs) - 1, int(p / 100 * len(xs)))], 3)


def dist(xs):
    return {"n": len(xs), "p50": pct(xs, 50), "p90": pct(xs, 90), "p99": pct(xs, 99),
            "min": round(min(xs), 3) if xs else None, "max": round(max(xs), 3) if xs else None}


def main(run):
    run = Path(run)
    evs = [json.loads(line) for line in open(run / "truth.jsonl")]
    commit_wall = {int(k): v for k, v in json.load(open(run / "commit_ts.json")).items()}
    # An interval that straddles a wall-clock step is meaningless; drop those events.
    bad = [e for e in evs if e["e"] < e["s"] or e["e"] - e["s"] > 1000]
    bad_reqs = {e["r"] for e in bad}
    evs = [e for e in evs if e["r"] not in bad_reqs]
    run_info = json.load(open(run / "run.json"))

    # Commit timestamps follow the wall clock, which steps and slews; harness
    # times do not. Each clock sample gives the wall-clock offset floored to a
    # millisecond, so the max over a +-50ms window recovers it to well under a
    # millisecond. Windows never cross a step (> 5ms jump between samples).
    samples = json.load(open(run / "clock.json"))
    seg, steps = [], []
    for i, (t, w) in enumerate(samples):
        if i and abs((w - t) - (samples[i - 1][1] - samples[i - 1][0])) > 5:
            steps.append({"t": t, "jump_ms": round((w - t) - (samples[i - 1][1] - samples[i - 1][0]), 1)})
        seg.append(len(steps))
    times = [t for t, _ in samples]
    smooth = []
    j0 = j1 = 0
    for i, (t, _) in enumerate(samples):
        while times[j0] < t - 50 or seg[j0] != seg[i]:
            j0 += 1
        j1 = max(j1, i)
        while j1 + 1 < len(samples) and times[j1 + 1] <= t + 50 and seg[j1 + 1] == seg[i]:
            j1 += 1
        smooth.append(max(samples[j][1] - samples[j][0] for j in range(j0, j1 + 1)))

    def offset_at(t):
        i = min(max(bisect.bisect_left(times, t), 0), len(times) - 1)
        return seg[i], smooth[i]

    commit_ts = {}
    for e in evs:
        if e["t"] == "upd" and commit_wall.get(e["xid"]) is not None:
            (sa, oa), (sb, _) = offset_at(e["s"]), offset_at(e["e"])
            if sa == sb:  # no step inside the UPDATE
                commit_ts[e["xid"]] = commit_wall[e["xid"]] - oa
    end_state = json.load(open(run / "end_state.json"))
    viols = [json.loads(line) for line in open(run / "violations.jsonl")] if (run / "violations.jsonl").exists() else []

    by_req = collections.defaultdict(dict)
    for e in evs:
        by_req[e["r"]][e["t"]] = e

    # Per key: committed versions in order, with commit timestamp and client-side interval.
    commits = collections.defaultdict(list)  # k -> [(version, commit_ts, upd_s, upd_e)]
    writer_del = {}                          # (k, version) -> the DEL issued by that version's writer
    for e in evs:
        if e["t"] == "upd":
            commits[e["k"]].append((e["v"], commit_ts.get(e["xid"]), e["s"], e["e"]))
            d = by_req[e["r"]].get("del")
            if d:
                writer_del[(e["k"], e["v"])] = d
    for k in commits:
        commits[k].sort()
    cts_by_kv = {(k, v): ts for k, cs in commits.items() for v, ts, _, _ in cs}
    ack_by_kv = {(k, v): ue for k, cs in commits.items() for v, _, _, ue in cs}

    def latest_before(k, t):
        """The newest version of key k whose commit timestamp is before t."""
        best = None
        for c in commits.get(k, []):
            if c[1] is not None and c[1] < t:
                best = c
        return best

    def truth_cause_of(sel, nxt):
        """Why a cache fill read an old version: where and when its SELECT ran."""
        if sel.get("tgt") == "replica":
            return "REPLICA_READ"
        if nxt and nxt[1] is not None and sel["s"] < nxt[1]:
            return "READ_BEFORE_COMMIT"
        return "READ_AFTER_COMMIT_TS"

    def first_newer(k, v):
        """The first version after v of key k, or None."""
        cs = commits.get(k, [])
        i = bisect.bisect_right([c[0] for c in cs], v)
        return cs[i] if i < len(cs) else None

    # ---- A. commit timestamp vs what clients see
    ack_after_commit = [ue - ts for k, cs in commits.items() for v, ts, us, ue in cs if ts is not None]
    commit_after_start = [ts - us for k, cs in commits.items() for v, ts, us, ue in cs if ts is not None]
    old_reads_after_commit_ts = []  # SELECT started after the newer version's commit ts, yet read the older one
    old_reads_after_ack = 0         # ... after the writer had already received its acknowledgement
    for e in evs:
        if e["t"] != "sel" or e.get("tgt") != "primary":
            continue
        nxt = first_newer(e["k"], e["v"])
        if nxt and nxt[1] is not None and e["s"] > nxt[1]:
            old_reads_after_commit_ts.append(e["s"] - nxt[1])
            if e["s"] > nxt[3]:
                old_reads_after_ack += 1

    # ---- B. stale SETs by truth: a SET whose version had already been superseded when it started
    stale_sets = []
    for e in evs:
        if e["t"] != "set":
            continue
        nxt = first_newer(e["k"], e["v"])
        if not nxt or nxt[1] is None or nxt[1] >= e["s"]:
            continue
        sel = by_req[e["r"]].get("sel")
        cause = truth_cause_of(sel, nxt)
        # Did it land after the invalidation of the newest committed version? Only then does it stick.
        lat = latest_before(e["k"], e["s"])
        d = writer_del.get((e["k"], lat[0])) if lat else None
        if d is None:
            order = "NO_DEL_YET"
        elif e["s"] > d["e"]:
            order = "AFTER_DEL"
        elif e["e"] < d["s"]:
            order = "BEFORE_DEL"
        else:
            order = "OVERLAP"
        stale_sets.append({"r": e["r"], "k": e["k"], "v": e["v"], "s": e["s"], "cause": cause, "order": order})

    # ---- C. stale reads served by truth: GET hits returning a version superseded before the GET started
    served = []
    for e in evs:
        if e["t"] != "get" or not e["hit"]:
            continue
        nxt = first_newer(e["k"], e["v"])
        if nxt and nxt[3] < e["s"]:  # writer had its ack before this read began
            served.append(e["s"] - nxt[1] if nxt[1] is not None else None)
    served = [x for x in served if x is not None]

    # ---- D. end state
    oracle_keys = {f"user:{s['k']}" for s in end_state if s["cache"] is not None and s["cache"] != s["db"]}
    viol_keys = {v["key"] for v in viols if v["severity"] == "error"}

    # ---- E. each tool violation against the truth
    sets_by_kv = collections.defaultdict(list)
    for e in evs:
        if e["t"] == "set":
            sets_by_kv[(e["k"], e["v"])].append(e)
    misses_by_req = {r: d["get"] for r, d in by_req.items() if "get" in d and not d["get"]["hit"]}
    rows = []
    for v in viols:
        k = int(v["key"].split(":")[1])
        cached_v = json.loads(v["cached"])["version"]
        latest_v = int(v["db_latest"]["version"])
        tc = cts_by_kv.get((k, latest_v))
        detect_at = tc + v["detected_ms_after_commit"] if tc is not None else None
        cands = [s for s in sets_by_kv.get((k, cached_v), []) if detect_at is None or s["s"] < detect_at]
        writer = max(cands, key=lambda s: s["s"]) if cands else None
        truth_cause, miss_rel, sel_rel = None, None, None
        if writer:
            sel = by_req[writer["r"]]["sel"]
            nxt = first_newer(k, cached_v)
            truth_cause = truth_cause_of(sel, nxt)
            miss = misses_by_req.get(writer["r"])
            if miss and tc is not None:
                miss_rel = round(miss["s"] - tc, 2)
            if tc is not None:
                sel_rel = round(sel["s"] - tc, 2)
        m = re.search(r"cache miss was at ([+-][\d.]+)ms", v["explanation"])
        tool_miss = float(m.group(1)) if m else None
        m2 = re.search(r"conn (\d+) (?:then wrote|refilled|wrote|missed)", v["explanation"])
        tool_conn = int(m2.group(1)) if m2 else None
        rows.append({
            "key": v["key"], "tool_cause": v["cause"], "truth_cause": truth_cause,
            "in_oracle": v["key"] in oracle_keys,
            "writer_conn": writer["conn"] if writer else None, "tool_conn": tool_conn,
            "truth_miss_ms": miss_rel, "tool_miss_ms": tool_miss, "truth_select_ms": sel_rel,
            "miss_attribution": None if tool_miss is None or miss_rel is None
            else ("same" if abs(tool_miss - miss_rel) <= 0.3 else "different request"),
        })

    summary = {
        "requests": len(by_req),
        "dropped_requests_clock_step": len(bad_reqs),
        "clock_steps": steps,
        "commit_ts_to_writer_ack_ms": dist(ack_after_commit),
        "update_start_to_commit_ts_ms": dist(commit_after_start),
        "old_reads_after_newer_commit_ts": {**dist(old_reads_after_commit_ts), "after_writer_ack": old_reads_after_ack},
        "stale_sets": {"total": len(stale_sets), "by_cause": dict(collections.Counter(s["cause"] for s in stale_sets)),
                       "by_order_vs_del": dict(collections.Counter(s["order"] for s in stale_sets)),
                       "keys": len({s["k"] for s in stale_sets})},
        "stale_reads_served_ms_after_commit": {**dist(served), "over_100ms": sum(x > 100 for x in served),
                                              "over_500ms": sum(x > 500 for x in served)},
        "end_state": {"oracle_stale_keys": len(oracle_keys), "tool_error_keys": len(viol_keys),
                      "oracle_detected": len(oracle_keys & viol_keys), "tool_only": sorted(viol_keys - oracle_keys)},
        "violations": rows,
        "cause_matrix": dict(collections.Counter(f"{r['tool_cause']} <- {r['truth_cause']}" for r in rows)),
        "miss_attribution": dict(collections.Counter(r["miss_attribution"] for r in rows)),
    }
    json.dump(summary, open(run / "analysis.json", "w"), indent=1)

    print(f"requests {summary['requests']}  dropped (clock step) {summary['dropped_requests_clock_step']}  steps {summary['clock_steps']}")
    print(f"commit ts -> writer ack (ms)        {summary['commit_ts_to_writer_ack_ms']}")
    print(f"old reads after newer commit ts     {summary['old_reads_after_newer_commit_ts']}")
    print(f"stale SETs (truth)                  {summary['stale_sets']}")
    print(f"stale reads served (truth)          {summary['stale_reads_served_ms_after_commit']}")
    print(f"end state                           {summary['end_state']}")
    print(f"cause matrix (tool <- truth)        {summary['cause_matrix']}")
    print(f"reader-miss attribution             {summary['miss_attribution']}")
    for r in rows:
        print("  ", r)


if __name__ == "__main__":
    main(sys.argv[1])
