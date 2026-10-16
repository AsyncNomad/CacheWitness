# Command Line

```text
cawit verify <SCENARIO> [OPTIONS]
```

Runs a scenario and judges every run.

| Option | Meaning |
|---|---|
| `<SCENARIO>` | Path to the scenario YAML file |
| `--runs <N>` | Number of runs; overrides `runs` in the scenario |
| `--out <PATH>` | Write the full report as JSON ([Report Formats](report-format.md#json)) |
| `--junit <PATH>` | Write a JUnit XML report, one test case per run ([Report Formats](report-format.md#junit)) |
| `--show-steps` | Print the steps the scenario runs (a pattern expanded) as YAML, then exit `0` without connecting to anything |
| `-h`, `--help` | Help |

`cawit --version` prints the version.

## Output

For each run, the outcome and each expectation, then a summary:

```text
run   1  PASS
          ok   not_stale (app_equals_db): served=…-1-b (cache miss) db=…-1-b
          ok   refilled (app_cached_equals_db): served=…-1-b (cache hit) db=…-1-b
run   2  NOT_REACHED
          sync point `refill:after-read` not reached within 10000ms

directus-stale-set: 2 runs  pass 1  fail 0  not_reached 1  error 0
  not_stale            passed 1/1
  refilled             passed 1/1
```

The per-expectation totals count only runs that were judged (`PASS` or `FAIL`).

## Exit codes

| Code | Meaning |
|---|---|
| `0` | every run passed |
| `1` | at least one run failed |
| `2` | no run failed, but some were discarded (`NOT_REACHED`, `ERROR`) or there were no runs; also any error that prevents the scenario from running (invalid file, unreachable PostgreSQL or Redis), printed as `cawit: <error>` |
