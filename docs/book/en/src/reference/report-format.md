# Report Formats

## JSON

`cawit verify --out report.json` writes:

```json
{
  "scenario": "directus-stale-set",
  "runs": [
    {
      "run": 1,
      "outcome": "FAIL",
      "expectations": [
        {
          "name": "not_stale",
          "check": "app_equals_db",
          "pass": false,
          "detail": "served=1a1422e8c34-1-a (cache hit) db=1a1422e8c34-1-b"
        }
      ]
    },
    {
      "run": 2,
      "outcome": "NOT_REACHED",
      "expectations": [],
      "note": "sync point `refill:after-read` not reached within 10000ms"
    }
  ]
}
```

| Field | Meaning |
|---|---|
| `scenario` | the scenario's `name` |
| `runs[].run` | run number, from 1 |
| `runs[].outcome` | `PASS`, `FAIL`, `NOT_REACHED`, or `ERROR` |
| `runs[].expectations[]` | the `expect` steps evaluated before the run ended, in order: `name`, `check`, `pass`, `detail` |
| `runs[].note` | present when the run ended early: why |

## JUnit

`cawit verify --junit report.xml` writes one `<testsuite>` named after the scenario, with one `<testcase>` per run:

```xml
<testsuite name="directus-stale-set" tests="2" failures="1" skipped="1">
  <testcase classname="directus-stale-set" name="run 1">
    <failure message="not_stale">FAIL not_stale (app_equals_db): served=… db=…</failure>
  </testcase>
  <testcase classname="directus-stale-set" name="run 2">
    <skipped message="NotReached">sync point `refill:after-read` not reached within 10000ms</skipped>
  </testcase>
</testsuite>
```

- `FAIL` runs carry `<failure>` with the names of the failed expectations.
- `NOT_REACHED` and `ERROR` runs are `<skipped>`: they neither pass nor fail the fix. Make CI fail on them through the exit code (`2`).
