# 명령줄

```text
cawit verify <SCENARIO> [OPTIONS]
```

시나리오를 실행하고 모든 회차를 판정합니다.

| 옵션 | 의미 |
|---|---|
| `<SCENARIO>` | 시나리오 YAML 파일 경로 |
| `--runs <N>` | 회차 수. 시나리오의 `runs`보다 우선 |
| `--out <PATH>` | 전체 리포트를 JSON으로 저장 ([리포트 형식](report-format.md#json)) |
| `--junit <PATH>` | JUnit XML 리포트 저장, 회차 하나가 테스트 케이스 하나 ([리포트 형식](report-format.md#junit)) |
| `--show-steps` | 시나리오가 실행할 단계(패턴은 펼친 결과)를 YAML로 출력하고, 아무 데도 연결하지 않은 채 `0`으로 종료 |
| `-h`, `--help` | 도움말 |

`cawit --version`은 버전을 출력합니다.

## 출력

회차마다 결과와 각 판정, 그리고 요약:

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

판정별 합계는 판정까지 간 회차(`PASS`, `FAIL`)만 셉니다.

## 종료 코드

| 코드 | 의미 |
|---|---|
| `0` | 모든 회차 통과 |
| `1` | 하나 이상의 회차 실패 |
| `2` | 실패는 없지만 버린 회차(`NOT_REACHED`, `ERROR`)가 있거나 회차가 없음. 시나리오를 실행할 수 없게 하는 오류(잘못된 파일, PostgreSQL이나 Redis에 닿지 않음)도 `cawit: <오류>`를 출력하고 `2` |
