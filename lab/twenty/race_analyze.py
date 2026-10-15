"""Find stale writes of the workspace entity cache in a race.sh run.

Usage: python3 -I lab/twenty/race_analyze.py <race dir>

Each workspace version is named race-<n>, so every cached value says which
version it holds. Two kinds of evidence:

  regression   in Redis execution order, a SET of race-k follows a SET of
               race-j with j > k. Needs no clock at all.
  stale SET    a SET of race-k at a time when race-L (L > k) had already been
               committed for at least MARGIN_MS. Compares Redis and WAL
               timestamps, so SETs near a wall-clock step are set aside.

For each stale SET, the window until the next DEL or SET of the key is how
long the cache served an outdated workspace; GET/MGET of the key inside the
window are stale reads that reached Redis (Twenty's in-process L1 cache
serves more reads that MONITOR cannot see).
"""

import bisect
import json
import re
import sys
from datetime import datetime
from pathlib import Path

WS = "20202020-1c25-4d02-bf25-6aeccf7ea419"
KEY = f"integration-tests:engine:core-entity:workspace:{WS}"
MARGIN_MS = 5


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
    ctimes = [c[0] for c in commits]

    def latest_committed(t):
        i = bisect.bisect_right(ctimes, t - MARGIN_MS)
        return commits[i - 1] if i else None

    def near_step(t0, t1):
        return any(t0 - 3500 <= s <= t1 + 3500 for s in steps)

    sets = [e for e in evs if e[2] == "set"]
    regressions, best = [], 0
    for e in sets:
        if e[3] < best:
            regressions.append({"order": e[0], "wrote": e[3], "after": best})
        best = max(best, e[3])

    stale = []
    for idx, e in enumerate(evs):
        if e[2] != "set":
            continue
        lc = latest_committed(e[1])
        if not lc or e[3] >= lc[1]:
            continue
        nxt = next((x for x in evs[idx + 1:] if x[2] in ("set", "del")), None)
        end_t = nxt[1] if nxt else None
        reads = sum(1 for x in evs[idx + 1:(evs.index(nxt) if nxt else len(evs))] if x[2] == "read")
        stale.append({
            "order": e[0], "wrote": e[3], "already_committed": lc[1],
            "ms_after_newer_commit": round(e[1] - lc[0], 1),
            "stale_for_ms": round(end_t - e[1], 1) if end_t else None,
            "redis_reads_while_stale": reads,
            "ended_by": nxt[2] if nxt else "end of run",
            "near_clock_step": near_step(lc[0], end_t or e[1]),
        })

    race = json.load(open(run / "race.json"))
    final_db = (run / "final-db.txt").read_text().strip()
    fc = (run / "final-cache.json").read_text().strip()
    final_cache = json.loads(fc).get("displayName") if fc else None
    clean = [s for s in stale if not s["near_clock_step"]]
    summary = {
        "writes_ok": race["writes_ok"], "reads_ok": race["reads"]["ok"], "commits_seen": len(commits),
        "cache_sets": len(sets), "cache_dels": sum(1 for e in evs if e[2] == "del"),
        "clock_steps": len(steps),
        "regressions": regressions,
        "stale_sets": len(stale), "stale_sets_not_near_step": len(clean),
        "stale_over_500ms": [s for s in clean if (s["stale_for_ms"] or 1e9) > 500],
        "final": {"db": final_db, "cache": final_cache, "stale_at_end": final_cache not in (None, final_db)},
        "stale": stale,
    }
    json.dump(summary, open(run / "race-analysis.json", "w"), indent=1)
    print(f"writes {summary['writes_ok']}  reads {summary['reads_ok']}  commits {len(commits)}  "
          f"cache SET {len(sets)}  DEL {summary['cache_dels']}  clock steps {len(steps)}")
    print(f"regressions (no clock needed): {len(regressions)}  {regressions[:5]}")
    print(f"stale SETs: {len(stale)}  (not near a clock step: {len(clean)}, stale > 500ms: {len(summary['stale_over_500ms'])})")
    for s in clean[:20]:
        print("  ", s)
    print("final:", summary["final"])


if __name__ == "__main__":
    main(sys.argv[1])
