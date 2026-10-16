# verify 엔진 설계 (2단계)

PLAN 13장 2단계의 설계다. 제품 정의(PLAN 0장, 2026-10-16)의 주력 기능인 verify 모드를 만든다. 원형은 1단계 실험의 `lab/twenty/sync.sh`, `sync-scenario.js`, `refill-delay.patch`다 (`docs/design/phase1-results.md` 5.8).

## 1. 목표와 관문

- **목표**: 알려진 캐시 경쟁 패턴을 매번 같은 순서로 일으키고, 수정 전 실패·수정 후 통과를 CI에서 보여준다.
- **2단계 관문** (PLAN 13장): 두 대상(Twenty + Node/TS 하나 더)에서 수정 전 매번 실패, 올바른 수정 매번 통과, 그럴듯하지만 틀린 수정을 실패로 판별. 훅을 거는 앱 코드 변경 10줄 이내.

## 2. 구성 요소

| 구성 요소 | 위치 | 역할 |
|---|---|---|
| 동기화 훅 (Node/TS) | `hooks/node/` | 앱의 재충전 경로에서 `syncPoint(point, key)` 한 줄. 의존성 없음 |
| 훅 프로토콜 | 이 문서 3장 | 실행기와 훅이 Redis 리스트로 주고받는 약속. 언어와 무관 |
| 시나리오 형식 | 이 문서 4장 | YAML. 요청, 장전, 도달 대기, 해제, 판정을 순서대로 적는다 |
| 실행기 | `crates/cawit-verify` | 시나리오를 회차만큼 돌리고 결과를 모은다 |
| CLI | `crates/cawit` | `cawit verify <시나리오.yaml>` |
| RESP 클라이언트 | `crates/cawit-resp` | 실행기용 최소 Redis 클라이언트. 이후 관측 엔진과 파서를 공유한다 |

## 3. 훅 프로토콜 v1

모든 키는 `cawit:` 접두사를 쓴다. 앱은 환경 변수 `CAWIT_SYNC_URL`(예: `redis://127.0.0.1:6379`)이 있을 때만 훅을 켠다. **없으면 훅은 아무 I/O도 하지 않고 바로 돌아간다.** 운영 환경에서 실수로 켜질 위험을 줄이기 위해서다.

| 단계 | 누가 | 명령 | 의미 |
|---|---|---|---|
| 장전 | 실행기 | `SET cawit:arm:<point> <token>\|<key 또는 *> PX 30000` | 다음에 `<point>`에 도달하는 호출 하나를 붙잡는다. `<key>`를 주면 그 키일 때만 |
| 소비 | 훅 | `GETDEL cawit:arm:<point>` | 장전 값이 없으면 바로 반환. 키가 맞지 않으면 값을 되돌려 놓고 반환 |
| 도달 알림 | 훅 | `RPUSH cawit:reached:<token> <JSON {point,key,pid}>` | 실행기가 이것을 조작 확인으로 쓴다 |
| 대기 | 훅 | `BLPOP cawit:release:<token> <초>` | 해제를 기다린다. 시간이 지나면 `RPUSH cawit:timeout:<token>` 후 진행 |
| 도달 대기 | 실행기 | `BLPOP cawit:reached:<token> <초>` | 시간 안에 오지 않으면 그 회차는 `NOT_REACHED` |
| 해제 | 실행기 | `RPUSH cawit:release:<token> go` | 붙잡힌 호출을 풀어 준다 |

- **토큰**: 회차마다 새로 만든다. 이전 회차의 남은 신호가 섞이지 않는다.
- **소비가 원자적이어야 한다**: 동시에 여러 요청이 같은 지점에 와도 붙잡히는 것은 하나다(`GETDEL`).
- **대기용 연결**: `BLPOP`은 연결을 막으므로 훅은 앱의 캐시 연결이 아닌 **자기 전용 연결**을 쓴다. 앱의 캐시 연결을 막으면 1단계에서 본 head-of-line 문제가 생긴다.

**기각한 대안**

| 대안 | 기각 이유 |
|---|---|
| 앱에 HTTP 제어 엔드포인트 추가 | 앱마다 라우팅을 고쳐야 한다. 훅 한 줄보다 변경이 크다 |
| 프록시에서 명령을 붙잡기 | 1단계에서 연결 단위 대기열이 결과를 왜곡함을 확인했다 |
| 파일·환경 변수로 신호 | 서버가 여러 대이거나 컨테이너가 다르면 공유가 어렵다. Redis는 대상 앱에 이미 있다 |
| 무작위 지연 주입 | 실행마다 결과가 갈린다(1단계 5.7). 결정적이지 않다 |

## 4. 시나리오 형식

```yaml
name: stale-set
runs: 20
apps:
  a: http://127.0.0.1:3000
  b: http://127.0.0.1:3001
redis: redis://127.0.0.1:6379
postgres: "host=127.0.0.1 user=postgres password=postgres dbname=test"

requests:            # 이름 붙인 HTTP 요청 템플릿. ${...}는 회차 변수
  rename:
    method: POST
    path: /metadata
    headers: { authorization: "Bearer ${env.TOKEN}" }
    json: { query: "mutation($n: String!) { updateWorkspace(data: { displayName: $n }) { id } }",
            variables: { n: "${value}" } }
  read:
    method: POST
    path: /metadata
    headers: { authorization: "Bearer ${env.TOKEN}" }
    json: { query: "{ currentWorkspace { id } }" }

state:               # 판정 대상: 캐시 값의 필드와 DB 값
  cache_key: "integration-tests:engine:core-entity:workspace:20202020-1c25-4d02-bf25-6aeccf7ea419:data"
  cache_field: displayName
  db_query: "select \"displayName\" from core.workspace where id = '20202020-1c25-4d02-bf25-6aeccf7ea419'"

steps:
  - http: { app: a, request: rename, vars: { value: "${run}-a" } }
  - arm: { point: "refill:after-read", as: r1 }
  - http: { app: a, request: read, async: pending_read }
  - reached: { sync: r1, timeout_ms: 10000 }
  - http: { app: b, request: rename, vars: { value: "${run}-b" } }
  - release: { sync: r1 }
  - await: { task: pending_read }
  - sleep: { ms: 100 }
  - expect: { name: not_stale, check: cache_not_stale }
  - http: { app: a, request: read }
  - sleep: { ms: 100 }
  - expect: { name: refilled, check: cache_equals_db }
```

- **단계는 레지스트리로 확장한다.** `http`, `arm`, `reached`, `release`, `await`, `sleep`, `expect`가 기본이다. 새 단계(SQL 실행, Redis 명령 등)는 코어를 고치지 않고 등록한다 (PLAN 6.2 원칙 7).
- **판정 검사도 레지스트리다.** `cache_not_stale`(캐시가 비어 있거나 DB 값과 같음), `cache_equals_db`(캐시가 DB 값과 같음), `cache_absent`.
- **회차 변수**: `${run}`(회차 번호와 실행 ID를 합친 고유 문자열), `${env.X}`(환경 변수), 단계 `vars`.

## 5. 결과와 종료 코드

| 회차 결과 | 조건 |
|---|---|
| `PASS` | 모든 `expect`가 통과 |
| `FAIL` | `expect` 하나 이상 실패 |
| `NOT_REACHED` | `reached` 단계가 시간 안에 신호를 받지 못함. 회차를 버린다 (조작 확인) |
| `ERROR` | 요청 실패 등. 회차를 버린다 |

| 종료 코드 | 조건 |
|---|---|
| 0 | 모든 회차 `PASS` |
| 1 | `FAIL`이 하나라도 있음 |
| 2 | `FAIL`은 없지만 `NOT_REACHED`나 `ERROR`가 있음 (판정 불가) |

결과는 회차별 JSON(`--out`)과 사람이 읽는 요약으로 낸다. JUnit은 2단계 안에서 추가한다.

## 6. 실행 환경

실행기 바이너리도 실험 환경과 같은 격리 네트워크 안에서 돈다 (`lab/twenty/isolation.sh`). 빌드는 로컬 cargo 캐시만으로(`--offline`) 한다. 새 크레이트가 필요해지면 네트워크 사용 허락을 먼저 받는다.

## 7. 사각지대

- 훅은 **알려 준 지점**에서만 순서를 통제한다. 재충전 경로가 여러 개면 지점도 여러 개 걸어야 한다. 빠뜨린 경로는 시험되지 않는다.
- 시나리오의 판정은 "그 키의 최종 상태"다. 프로세스 안 캐시(L1, 메모이저)가 낡은 값을 응답하는 문제는 "다시 채워지는가" 검사로 간접적으로만 잡는다. 1단계에서 리스만 넣은 수정이 이렇게 잡혔다.
- 서버 두 대를 띄우는 시나리오는 앱이 수평 확장을 지원해야 한다. 서버 한 대 안에서의 경쟁 시나리오는 3단계에서 다룬다.
