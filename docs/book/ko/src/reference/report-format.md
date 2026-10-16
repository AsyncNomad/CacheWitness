# 리포트 형식

## JSON

`cawit verify --out report.json`이 쓰는 내용:

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

| 필드 | 의미 |
|---|---|
| `scenario` | 시나리오의 `name` |
| `runs[].run` | 회차 번호, 1부터 |
| `runs[].outcome` | `PASS`, `FAIL`, `NOT_REACHED`, `ERROR` |
| `runs[].expectations[]` | 회차가 끝나기 전에 평가된 `expect` 단계, 순서대로: `name`, `check`, `pass`, `detail` |
| `runs[].note` | 회차가 일찍 끝났을 때만: 이유 |

## JUnit

`cawit verify --junit report.xml`은 시나리오 이름으로 `<testsuite>` 하나를 쓰고, 회차마다 `<testcase>` 하나를 씁니다.

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

- `FAIL` 회차는 실패한 판정 이름과 함께 `<failure>`.
- `NOT_REACHED`, `ERROR` 회차는 `<skipped>`. 수정의 통과도 실패도 아니기 때문입니다. CI가 이 경우에도 실패하게 하려면 종료 코드(`2`)를 쓰십시오.
