"""Phase 1: what Twenty's integration tests did to its Redis entity cache.

Usage: python3 -I lab/twenty/analyze.py <record dir>

Inputs from record.sh: monitor.log (Redis MONITOR) and wal.log (test_decoding).
Reports
  1. which Redis commands touch which key families (v0 command coverage),
  2. key patterns with ids normalised (mapping effort),
  3. for the core entity cache (user, workspace, user-workspace): commits that
     changed a row while its cache entry was present, and whether an
     invalidation or rewrite followed. Entries left in place whose cached
     fields differ from the committed row are missing-invalidation candidates.

MONITOR shows commands, not replies, so cache state is reconstructed from the
writes it saw; TTL expiry is ignored (the entity cache uses long TTLs).
"""

import collections
import json
import re
import sys
from datetime import datetime
from pathlib import Path

UUID = r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}"
ENTITY_TABLES = {"user": 'core."user"', "workspace": "core.workspace", "user-workspace": 'core."userWorkspace"'}
WRITE_CMDS = {"SET", "MSET", "SETEX", "PSETEX", "SETNX", "GETSET", "GETDEL", "DEL", "UNLINK", "EXPIRE", "PEXPIRE",
              "HSET", "HDEL", "SADD", "SREM", "INCR", "INCRBY", "DECR", "LPUSH", "RPUSH", "ZADD", "ZREM", "FLUSHALL",
              "FLUSHDB", "EVAL", "EVALSHA", "FCALL"}


def parse_monitor_line(line):
    m = re.match(r"(\d+\.\d+) \[(\d+) ([^\]]+)\] (.*)$", line)
    if not m:
        return None
    ts, db, client, rest = float(m.group(1)) * 1000, int(m.group(2)), m.group(3), m.group(4)
    args, i = [], 0
    while i < len(rest):
        if rest[i] == '"':
            i += 1
            buf = bytearray()
            while rest[i] != '"':
                if rest[i] == "\\":
                    nxt = rest[i + 1]
                    if nxt == "x":
                        buf.append(int(rest[i + 2:i + 4], 16))
                        i += 4
                        continue
                    buf += {"n": b"\n", "r": b"\r", "t": b"\t", "a": b"\a", "b": b"\b"}.get(nxt, nxt.encode())
                    i += 2
                    continue
                buf += rest[i].encode()
                i += 1
            args.append(buf.decode("utf-8", "replace"))
        i += 1
    return {"t": ts, "db": db, "client": client, "cmd": args[0].upper() if args else "", "args": args[1:]}


def keys_of(cmd, args):
    if cmd in ("MSET",):
        return args[0::2]
    if cmd in ("MGET", "DEL", "UNLINK", "EXISTS", "TOUCH"):
        return args
    if cmd in ("EVAL", "EVALSHA", "FCALL"):
        n = int(args[1]) if len(args) > 1 and args[1].isdigit() else 0
        return args[2:2 + n]
    if cmd in ("FLUSHALL", "FLUSHDB", "PING", "INFO", "CLIENT", "SELECT", "HELLO", "AUTH", "MULTI", "EXEC",
               "SCAN", "KEYS", "CONFIG", "MONITOR", "SCRIPT", "QUIT", "PUBLISH", "SUBSCRIBE", "PSUBSCRIBE"):
        return []
    return args[:1]


def family(key):
    k = re.sub(UUID, "{uuid}", key)
    k = re.sub(r"(?<=[:/\-])\d+(?=$|[:/\-])", "{n}", k)
    return k


def parse_cols(s):
    row, i = {}, 0
    while i < len(s):
        while i < len(s) and s[i] == " ":
            i += 1
        j = s.find("[", i)
        if j < 0:
            break
        name = s[i:j].strip('"')
        k = s.find("]:", j)
        i = k + 2
        if i < len(s) and s[i] == "'":
            i += 1
            val = []
            while i < len(s):
                if s[i] == "'":
                    if i + 1 < len(s) and s[i + 1] == "'":
                        val.append("'")
                        i += 2
                        continue
                    i += 1
                    break
                val.append(s[i])
                i += 1
            row[name] = "".join(val)
        else:
            k = s.find(" ", i)
            k = len(s) if k < 0 else k
            row[name] = s[i:k]
            i = k
    return row


def parse_wal(path):
    commits, txn = [], []
    for line in open(path, encoding="utf-8", errors="replace"):
        line = line.rstrip("\n")
        if line.startswith("BEGIN"):
            txn = []
        elif line.startswith("COMMIT"):
            m = re.search(r"\(at (.+)\)", line)
            ts = datetime.strptime(m.group(1) + "00", "%Y-%m-%d %H:%M:%S.%f%z").timestamp() * 1000 if m else None
            commits.append({"t": ts, "changes": txn})
        elif line.startswith("table "):
            m = re.match(r"table (.+?): (INSERT|UPDATE|DELETE): (.*)$", line)
            if not m:
                continue
            table, op, body = m.groups()
            if op == "UPDATE" and "new-tuple: " in body:
                body = body.split("new-tuple: ", 1)[1]
            if op == "DELETE" and body.startswith("old-key: "):
                body = body[len("old-key: "):]
            txn.append({"table": table, "op": op, "row": parse_cols(body)})
    return commits


def norm(v):
    """Compare cached JSON values with test_decoding text loosely."""
    if v is None:
        return "null"
    if isinstance(v, bool):
        return "t" if v else "f"
    if isinstance(v, (dict, list)):
        return json.dumps(v, sort_keys=True, separators=(",", ":"))
    s = str(v)
    if s in ("true", "false"):
        return s[0]
    if s[:2] in ('{"', "[{", '["') or s in ("{}", "[]"):
        try:  # json/jsonb column text from test_decoding
            return json.dumps(json.loads(s), sort_keys=True, separators=(",", ":"))
        except ValueError:
            pass
    if s.startswith("{") and s.endswith("}") and not s.startswith('{"'):
        # PostgreSQL array text from test_decoding, e.g. {a,b}
        inner = s[1:-1]
        return json.dumps([x.strip('"') for x in inner.split(",")] if inner else [], separators=(",", ":"))
    return s


def ts_like(s):
    return bool(re.match(r"\d{4}-\d{2}-\d{2}[ T]\d{2}:\d{2}", s or ""))


def main(run):
    run = Path(run)
    commits = parse_wal(run / "wal.log")

    # Stream the MONITOR log: count everything, keep events only for entity keys.
    rx = re.compile(r"^(.*?)engine:core-entity:(user|workspace|user-workspace):(" + UUID + r"):data$")
    by_cmd = collections.Counter()
    fam_cmd = collections.defaultdict(collections.Counter)
    clients = collections.Counter()
    state = {}           # key -> (value json or None, t)
    events = collections.defaultdict(list)  # key -> [(t, kind, value)]
    n_ops = 0
    # MONITOR lines are in Redis execution order but stamped with the wall
    # clock, which steps (WSL2: about -2.8s every few tens of seconds). A
    # timestamp that goes backwards marks a step; findings near one are
    # unreliable and are reported separately.
    steps, prev_t = [], None
    for line in open(run / "monitor.log", encoding="utf-8", errors="replace"):
        o = parse_monitor_line(line)
        if not o or o["cmd"] == "MONITOR":
            continue
        if prev_t is not None and o["t"] < prev_t - 500:
            steps.append(o["t"])
        prev_t = o["t"]
        n_ops += 1
        by_cmd[o["cmd"]] += 1
        clients[o["client"]] += 1
        for k in keys_of(o["cmd"], o["args"]):
            fam_cmd[family(k)][o["cmd"]] += 1
        cmd, args = o["cmd"], o["args"]
        if cmd == "MSET":
            pairs = list(zip(args[0::2], args[1::2]))
        elif cmd in ("SET", "SETEX", "PSETEX") and args:
            pairs = [(args[0], args[2] if cmd in ("SETEX", "PSETEX") else args[1])]
        elif cmd in ("DEL", "UNLINK"):
            pairs = [(k, None) for k in args]
        elif cmd == "FLUSHALL" or cmd == "FLUSHDB":
            for k in list(state):
                events[k].append((o["t"], "flush", None))
            state.clear()
            continue
        elif cmd in ("MGET", "GET"):
            for k in args:
                if rx.match(k):
                    events[k].append((o["t"], "read", state.get(k, (None,))[0]))
            continue
        else:
            continue
        for k, v in pairs:
            if rx.match(k):
                state[k] = (v, o["t"])
                events[k].append((o["t"], "del" if v is None else "set", v))

    # find the key prefix actually used for entity keys
    prefixes = collections.Counter(rx.match(k).group(1) for k in events)

    findings, checked = [], collections.Counter()
    for c in commits:
        for ch in c["changes"]:
            name = next((n for n, tbl in ENTITY_TABLES.items() if ch["table"] == tbl), None)
            if not name or "id" not in ch["row"]:
                continue
            checked[(name, ch["op"])] += 1
            for prefix in prefixes:
                key = f"{prefix}engine:core-entity:{name}:{ch['row']['id']}:data"
                evs = events.get(key, [])
                before = [e for e in evs if e[0] <= c["t"] and e[1] in ("set", "del", "flush")]
                if not before or before[-1][1] != "set":
                    continue  # not cached at commit time
                cached = before[-1][2]
                after = [e for e in evs if e[0] > c["t"] and e[1] in ("set", "del", "flush")]
                fixed_at = after[0][0] - c["t"] if after else None
                stale_reads = sum(1 for e in evs if e[0] > c["t"] and e[1] == "read" and (not after or e[0] < after[0][0]))
                try:
                    cached_obj = json.loads(cached)
                except ValueError:
                    cached_obj = None
                diffs = []
                if isinstance(cached_obj, dict) and ch["op"] == "UPDATE":
                    for f, v in cached_obj.items():
                        if f in ch["row"] and isinstance(v, list) and ch["row"][f] == "{}" and not v:
                            continue  # empty PostgreSQL array vs empty JSON list
                        if f in ch["row"] and norm(v) != norm(ch["row"][f]) and not (ts_like(ch["row"][f]) and ts_like(str(v))):
                            diffs.append(f)
                    # A soft delete changes only deletedAt, which the entity cache does not store.
                    if ch["row"].get("deletedAt", "null") != "null" and "deletedAt" not in cached_obj:
                        diffs.append("deletedAt (soft delete, not cached)")
                if ch["op"] == "DELETE":
                    diffs.append("(row deleted)")
                findings.append({
                    "entity": name, "id": ch["row"]["id"], "op": ch["op"], "commit_t": c["t"],
                    "invalidated_after_ms": round(fixed_at, 1) if fixed_at is not None else None,
                    "stale_reads_before_fix": stale_reads, "differing_fields": diffs,
                })

    prev_c = None
    for c in commits:
        if prev_c is not None and c["t"] is not None and c["t"] < prev_c - 500:
            steps.append(c["t"])
        prev_c = c["t"] if c["t"] is not None else prev_c

    def near_step(f):
        end = f["commit_t"] + (f["invalidated_after_ms"] or 0)
        return any(f["commit_t"] - 3500 <= st <= end + 3500 for st in steps)

    for f in findings:
        f["near_clock_step"] = near_step(f)
    never = [f for f in findings if f["invalidated_after_ms"] is None]
    slow = [f for f in findings if f["invalidated_after_ms"] is not None and f["invalidated_after_ms"] > 1000]
    slow_clean = [f for f in slow if not f["near_clock_step"]]
    summary = {
        "redis_ops": n_ops, "redis_clients": len(clients), "commits": len(commits),
        "commands": dict(by_cmd.most_common()),
        "families": {f: dict(c) for f, c in sorted(fam_cmd.items(), key=lambda x: -sum(x[1].values()))[:40]},
        "entity_key_prefixes": dict(prefixes),
        "entity_row_changes": {f"{n}:{op}": v for (n, op), v in checked.items()},
        "cached_at_commit": len(findings),
        "never_invalidated": never,
        "invalidated_after_1s": slow,
        "invalidated_after_1s_not_near_clock_step": slow_clean,
        "clock_steps_seen": len(steps),
        "invalidation_delay_ms": sorted(f["invalidated_after_ms"] for f in findings if f["invalidated_after_ms"] is not None),
    }
    json.dump(summary, open(run / "analysis.json", "w"), indent=1)
    print(f"redis ops {n_ops}  clients {len(clients)}  commits {len(commits)}")
    print("commands:", dict(by_cmd.most_common(25)))
    print("\ntop key families:")
    for f, c in list(summary["families"].items())[:25]:
        print(f"  {sum(c.values()):6d}  {f}  {dict(c)}")
    print("\nentity key prefixes:", summary["entity_key_prefixes"])
    print("entity row changes in WAL:", summary["entity_row_changes"])
    print(f"clock steps seen: {len(steps)}")
    print(f"entity rows changed while cached: {len(findings)}  never invalidated: {len(never)}  "
          f"invalidated >1s later: {len(slow)} (of which not near a clock step: {len(slow_clean)})")
    for f in never + slow:
        print("  ", f)


if __name__ == "__main__":
    main(sys.argv[1])
