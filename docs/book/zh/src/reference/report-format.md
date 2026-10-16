# 报告格式

## JSON

`cawit verify --out report.json` 写出：

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

| 字段 | 含义 |
|---|---|
| `scenario` | 场景的 `name` |
| `runs[].run` | 轮次编号，从 1 开始 |
| `runs[].outcome` | `PASS`、`FAIL`、`NOT_REACHED` 或 `ERROR` |
| `runs[].expectations[]` | 该轮结束前评估过的 `expect` 步骤，按顺序：`name`、`check`、`pass`、`detail` |
| `runs[].note` | 仅当该轮提前结束时出现：原因 |

## JUnit

`cawit verify --junit report.xml` 写出一个以场景命名的 `<testsuite>`，每一轮一个 `<testcase>`：

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

- `FAIL` 的轮次带有 `<failure>`，其中列出失败的判定名称。
- `NOT_REACHED` 和 `ERROR` 的轮次是 `<skipped>`：它们既不能证明修复通过，也不能证明失败。请通过退出码（`2`）让 CI 在这种情况下失败。
