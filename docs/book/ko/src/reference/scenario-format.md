# 시나리오 형식

시나리오는 YAML 파일입니다. 모든 단계에서 모르는 키는 오류이므로, 오타는 무시되지 않고 로드할 때 실패합니다.

```yaml
name: directus-stale-set          # 필수
runs: 20                          # 기본 1. --runs가 우선
apps:                             # 필수: 이름 붙인 기본 URL (http://만)
  a: http://127.0.0.1:8055
  b: http://127.0.0.1:8056
redis: redis://127.0.0.1:6379     # 필수: 프로토콜 + 캐시 읽기
postgres: "host=127.0.0.1 user=postgres password=postgres dbname=directus"

requests: { ... }                 # 이름 붙인 요청 템플릿
state: { ... }                    # 판정이 비교하는 것
pattern: { ... }                  # 패턴이나...
# steps: [ ... ]                  # ...직접 쓴 단계
```

## 최상위

| 키 | 타입 | 필수 | 의미 |
|---|---|---|---|
| `name` | 문자열 | 예 | 출력 이름, JUnit 스위트 이름 |
| `runs` | 정수 | 아니오 (1) | 회차 수 |
| `apps` | 이름 → URL | 예 | 애플리케이션 서버. `http://host:port`만 (TLS 없음) |
| `redis` | 문자열 | 예 | 훅 프로토콜과 `cache_key` 읽기용 `redis://host[:port][/db]`. 애플리케이션의 `CAWIT_SYNC_URL`과 같은 Redis여야 함 |
| `postgres` | 문자열 | 판정에 | libpq key=value 연결 문자열 |
| `requests` | 이름 → [요청](#요청) | 사실상 예 | 요청 템플릿 |
| `state` | [상태](#상태) | 판정에 | 판정이 비교하는 것 |
| `pattern` | 키 하나짜리 맵 | 둘 중 하나 | [패턴](../guide/patterns.md)과 매개변수 |
| `steps` | 리스트 | 둘 중 하나 | 직접 쓴 [단계](#단계) |

`pattern`과 `steps` 중 정확히 하나가 있어야 합니다.

## 요청

| 키 | 타입 | 기본값 | 의미 |
|---|---|---|---|
| `method` | 문자열 | `POST` | HTTP 메서드 |
| `path` | 문자열 | 필수 | 경로와 쿼리 |
| `headers` | 맵 | `{}` | 추가 헤더 |
| `json` | 아무 값 | 없음 | JSON 본문. `content-type: application/json`과 함께 보냄 |
| `error_field` | 문자열 | 없음 | 응답 JSON에 이 최상위 필드가 null이 아닌 값으로 있으면 오류로 봄 (GraphQL: `errors`) |

상태 400 이상은 오류입니다. 오류는 단계가 `allow_error`를 주지 않는 한 회차를 `ERROR`로 끝냅니다.

### 변수

`path`, 헤더 값, `json` 안의 모든 문자열에 `${...}`를 쓸 수 있습니다.

| 변수 | 값 |
|---|---|
| `${run}` | 회차마다 고유: 한 번의 실행에서 모든 회차가 공유하는 실행 ID, 대시, 회차 번호 |
| `${run_index}` | 회차 번호, 1부터 |
| `${env.NAME}` | `cawit` 프로세스의 환경 변수. 없으면 오류 |
| `${name}` | 단계의 `vars` 항목 (그 값에도 `${run}`, `${env.X}`를 쓸 수 있음) |

모르는 변수는 오류입니다. `arm` 단계의 `key`도 같은 방식으로 처리되고, `state`의 값은 그대로 씁니다.

## 상태

| 키 | 타입 | 의미 |
|---|---|---|
| `db_query` | 문자열 | 첫 행의 첫 열에 커밋된 값을 텍스트로 돌려주는 SQL (`::text`로 변환) |
| `cache_key` | 문자열 | 캐시 항목의 Redis 키 |
| `cache_field` | 문자열 | 항목이 JSON 객체일 때 꺼낼 최상위 필드 |
| `app_view` | 맵 | 대신 애플리케이션을 거쳐 읽음 (아래) |

`app_view`:

| 키 | 기본값 | 의미 |
|---|---|---|
| `app` | 필수 | 요청을 보낼 앱 |
| `request` | 필수 | 요청 템플릿 |
| `field` | 필수 | 응답 본문의 JSON pointer, 예: `/data/title` |
| `hit_header` | 없음 | 캐시가 응답했는지 알려 주는 헤더 |
| `hit_value` | `HIT` | 적중을 뜻하는 `hit_header` 값 |

## 패턴

[경쟁 패턴](../guide/patterns.md)을 보십시오. 매개변수:

| 매개변수 | `stale-set` | `failed-invalidation` | `invalidate-before-commit` | 기본값 |
|---|---|---|---|---|
| `reader` | 필수 | | 필수 | |
| `writer` | 필수 | | 필수 | |
| `app` | | 필수 | | |
| `write`, `read` | 필수 | 필수 | 필수 | |
| `value_var` | ✓ | ✓ | ✓ | `value` |
| `point` | ✓ | ✓ | ✓ | `refill:after-read` / `invalidate:before` / `write:before-commit` |
| `key` | ✓ | ✓ | ✓ | 아무 키 |
| `hold_ms` | ✓ | | ✓ | `0` |
| `settle_ms` | ✓ | ✓ | ✓ | `100` |
| `within_ms` | ✓ | ✓ | ✓ | 엄격 |
| `reached_timeout_ms` | ✓ | ✓ | ✓ | `10000` |

## 단계

각 단계는 키 하나짜리 맵, `{ <종류>: { <매개변수> } }`입니다.

### `http`

| 매개변수 | 기본값 | 의미 |
|---|---|---|
| `app` | 필수 | 앱 이름 |
| `request` | 필수 | 요청 템플릿 이름 |
| `vars` | `{}` | 추가 템플릿 변수 |
| `async` | 없음 | 이 작업 이름으로 백그라운드 실행. `await`로 수거 |
| `allow_error` | `false` | 오류 응답이 회차를 끝내지 않음 |

### `arm`

| 매개변수 | 기본값 | 의미 |
|---|---|---|
| `point` | 필수 | 동기화 지점 이름 |
| `key` | 아무 키 | 이 키의 호출만 가져감 |
| `action` | `hold` | `hold`(해제까지 대기) 또는 `fail`(한 번 예외) |
| `as` | 필수 | `reached`, `release`가 쓰는 별칭 |

아무도 가져가지 않으면 장전은 30초 뒤 만료됩니다.

### `reached`

| 매개변수 | 기본값 | 의미 |
|---|---|---|
| `sync` | 필수 | 별칭 |
| `timeout_ms` | `10000` | 기다리는 시간. 지나면 회차는 `NOT_REACHED` |

### `release`

| 매개변수 | 의미 |
|---|---|
| `sync` | 별칭. 애플리케이션에서 붙잡기가 이미 시간 초과됐다면 회차는 `NOT_REACHED` |

### `await`, `sleep`

`await: { task: <이름> }`은 백그라운드 요청을 기다립니다(오류를 허용하지 않았다면 그 오류가 회차를 끝냄). `sleep: { ms: <n> }`은 기다립니다.

### `expect`

| 매개변수 | 기본값 | 의미 |
|---|---|---|
| `name` | 필수 | 출력과 리포트의 이름 |
| `check` | 필수 | 판정 (아래) |
| `within_ms` | 엄격 | 50ms마다 다시 확인하고, 이 시간 안에 통과하면 통과 |

## 판정

| 판정 | 필요한 것 | 통과 조건 |
|---|---|---|
| `cache_not_stale` | `cache_key`, `postgres` | 항목이 없거나 DB와 같음 |
| `cache_equals_db` | `cache_key`, `postgres` | 항목이 있고 DB와 같음 |
| `cache_absent` | `cache_key` | 항목이 없음 |
| `app_equals_db` | `app_view`, `postgres` | 애플리케이션이 DB 값을 응답 |
| `app_cached_equals_db` | `hit_header`가 있는 `app_view`, `postgres` | 그리고 응답이 캐시 적중 |

## 회차의 끝

무슨 일이 있었든 회차는 동기화 지점 장전을 풀고, 아직 붙잡힌 것을 해제하고, 백그라운드 요청을 최대 10초 기다리고 끝납니다. 그래서 한 회차가 다음 회차를 위해 애플리케이션을 막아 두는 일이 없습니다.

## 직접 쓴 예

`stale-set` 패턴을 풀어 쓴 것입니다.

```yaml
steps:
  - http: { app: a, request: update, vars: { value: "${run}-a" } }
  - sleep: { ms: 50 }
  - arm: { point: "refill:after-read", key: /items/articles/1, as: refill }
  - http: { app: a, request: read, async: held_read }
  - reached: { sync: refill, timeout_ms: 10000 }
  - http: { app: b, request: update, vars: { value: "${run}-b" } }
  - release: { sync: refill }
  - await: { task: held_read }
  - sleep: { ms: 100 }
  - expect: { name: not_stale, check: app_equals_db }
  - expect: { name: refilled, check: app_cached_equals_db }
```
