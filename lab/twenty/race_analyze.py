"""Find stale writes of the workspace entity cache in a race.sh run.

Usage: python3 -I lab/twenty/race_analyze.py <race dir>

Each workspace version is named race-<n> (unique across runs), so every
cached value says which version it holds. The server deletes the cache entry
after committing a rename and before answering, so when the i-th DEL of this
run executes in Redis, at least i renames of this run have committed. The
WAL gives the commit order. That gives a clock-free test, valid even with
several concurrent writers:

  stale SET    a SET of value v, after i DELs, where v is not the last of the
               first i committed values (some newer value had committed).
  regression   a SET whose value committed earlier than a value SET before it.

Timestamps are used only to measure how long each stale value stayed (until
the next DEL or SET of the key); a wall-clock step inside that window makes
the duration unreliable and is flagged. GET/MGET of the key inside the window
are stale reads that reached Redis (Twenty's in-process L1 cache serves more
reads that MONITOR cannot see).
"""

import json
import re
import sys
from datetime import datetime
from pathlib import Path

WS = "20202020-1c25-4d02-bf25-6aeccf7ea419"
KEY = f"integration-tests:engine:core-entity:workspace:{WS}"


def monitor_events(path):
    """(order, wall ms, kind, version) for the workspace entity keys, in Redis order."""
    rx_line = re.compile(r"^(\d+\.\d+) \[\d+ [^\]]+\] \"(\w+)\"")
    out, order, prev_t, steps = [], 0, None, []
    for line in open(path, encoding="utf-8", errors="replace"):
        m = rx_line.match(line)
        if not m:
            continue
        t = float(m.group(1)) * 1000
        if prev_t is not None and t < prev_t - 500:
            steps.append(t)
        prev_t = t
        if KEY not in line:
            continue
        order += 1
        cmd = m.group(2).upper()
        if cmd == "SET" and f'{KEY}:data"' in line:
            v = re.search(r'displayName\\":\\"race-(\d+)', line)
            out.append((order, t, "set", int(v.group(1)) if v else 0))
        elif cmd in ("DEL", "UNLINK") and f"{KEY}:data" in line:
            out.append((order, t, "del", None))
        elif cmd in ("GET", "MGET"):
            out.append((order, t, "read", None))
    return out, steps


def wal_commits(path):
    """[(commit wall ms, version)] for workspace renames, in commit order."""
    commits, pending = [], []
    for line in open(path, encoding="utf-8", errors="replace"):
        if line.startswith("BEGIN"):
            pending = []
        elif line.startswith("table core.workspace: UPDATE") and WS in line:
            v = re.search(r"\"displayName\"\[[^\]]*\]:'race-(\d+)'", line)
            if v:
                pending.append(int(v.group(1)))
        elif line.startswith("COMMIT") and pending:
            ts = re.search(r"\(at (.+)\)", line).group(1)
            t = datetime.strptime(ts + "00", "%Y-%m-%d %H:%M:%S.%f%z").timestamp() * 1000
            commits.append((t, max(pending)))
    return commits


def main(run):
    run = Path(run)
    evs, steps = monitor_events(run / "monitor.log")
    commits = wal_commits(run / "wal.log")
    for i in range(1, len(commits)):
        if commits[i][0] < commits[i - 1][0] - 500:
            steps.append(commits[i][0])
    def near_step(t0, t1):
        return any(t0 - 3500 <= s <= t1 + 3500 for s in steps)

    race = json.load(open(run / "race.json"))
    base = race.get("base", 0)
    sets = [e for e in evs if e[2] == "set"]
    regressions, best = [], -1
    cpos = {v: i for i, (_, v) in enumerate(commits)}
    for e in sets:
        if e[3] not in cpos:
            continue  # value left over from before this run
        if cpos[e[3]] < best:
            regressions.append({"order": e[0], "wrote": e[3] - base, "commit_pos": cpos[e[3]], "after_commit_pos": best})
        best = max(best, cpos[e[3]])

    # Clock-free: after the i-th DEL of this run, the first i committed values
    # (WAL order) have all committed. That holds only if every DEL came from
    # one rename, so check the counts.
    n_dels = sum(1 for e in evs if e[2] == "del")
    attribution_ok = n_dels == race["writes_ok"] == len(commits)
    pos = {v: i for i, (_, v) in enumerate(commits)}  # commit position of each value
    stale, dels_seen = [], 0
    for idx, e in enumerate(evs):
        if e[2] == "del":
            dels_seen += 1
            continue
        if e[2] != "set" or e[3] < base or e[3] not in pos:
            continue  # value left over from before this run
        if dels_seen == 0 or pos[e[3]] >= dels_seen - 1:
            continue
        committed = commits[dels_seen - 1][1]  # last of the values certainly committed
        nxt = next((x for x in evs[idx + 1:] if x[2] in ("set", "del")), None)
        stop = evs.index(nxt) if nxt else len(evs)
        dur = round(nxt[1] - e[1], 1) if nxt else None
        stale.append({
            "order": e[0], "wrote": e[3] - base, "already_committed": committed - base,
            "commits_behind": dels_seen - 1 - pos[e[3]],
            "stale_for_ms": dur,
            "duration_unreliable": dur is not None and (dur < 0 or near_step(e[1], nxt[1])),
            "redis_reads_while_stale": sum(1 for x in evs[idx + 1:stop] if x[2] == "read"),
            "ended_by": nxt[2] if nxt else "end of run",
        })

    final_db = (run / "final-db.txt").read_text().strip()
    fc = (run / "final-cache.json").read_text().strip()
    final_cache = json.loads(fc).get("displayName") if fc else None
    clean = [s for s in stale if not s["duration_unreliable"]]
    summary = {
        "writes_ok": race["writes_ok"], "reads_ok": race["reads"]["ok"], "commits_seen": len(commits),
        "cache_sets": len(sets), "cache_dels": n_dels, "del_attribution_ok": attribution_ok,
        "clock_steps": len(steps),
        "regressions": regressions,
        "stale_sets": len(stale), "stale_sets_reliable_duration": len(clean),
        "stale_over_500ms": [s for s in clean if s["stale_for_ms"] is None or s["stale_for_ms"] > 500],
        "final": {"db": final_db, "cache": final_cache, "stale_at_end": final_cache not in (None, final_db)},
        "stale": stale,
    }
    json.dump(summary, open(run / "race-analysis.json", "w"), indent=1)
    print(f"writes {summary['writes_ok']}  reads {summary['reads_ok']}  commits {len(commits)}  "
          f"cache SET {len(sets)}  DEL {n_dels}  clock steps {len(steps)}")
    if not attribution_ok:
        print("WARNING: DEL count differs from write count; clock-free stale detection is unreliable")
    print(f"regressions (no clock needed): {len(regressions)}  {regressions[:5]}")
    print(f"stale SETs (clock-free): {len(stale)}  (duration reliable: {len(clean)}, stale > 500ms: {len(summary['stale_over_500ms'])})")
    for s in stale[:20]:
        print("  ", s)
    print("final:", summary["final"])


if __name__ == "__main__":
    main(sys.argv[1])
