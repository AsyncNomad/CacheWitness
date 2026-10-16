# verify 엔진 설계 (2단계)

PLAN 13장 2단계의 설계다. 제품 정의(PLAN 0장, 2026-10-16)의 주력 기능인 verify 모드를 만든다. 원형은 1단계 실험의 `sync.sh`, `sync-scenario.js`(git 기록에만 남음), `refill-delay.patch`다 (`docs/design/phase1-results.md` 5.8).

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
| 소비 | 훅 | `EVAL` 한 번: 장전 값이 있고 키가 `*`이거나 자기 키와 같을 때만 `DEL`하고 토큰을 받는다 | 아니면 장전을 건드리지 않고 바로 반환 |
| 도달 알림 | 훅 | `RPUSH cawit:reached:<token> <JSON {point,key,pid}>` | 실행기가 이것을 조작 확인으로 쓴다 |
| 대기 | 훅 | `BLPOP cawit:release:<token> <초>` | 해제를 기다린다. 시간이 지나면 `RPUSH cawit:timeout:<token>` 후 진행 |
| 도달 대기 | 실행기 | `BLPOP cawit:reached:<token> <초>` | 시간 안에 오지 않으면 그 회차는 `NOT_REACHED` |
| 해제 | 실행기 | `RPUSH cawit:release:<token> go` | 붙잡힌 호출을 풀어 준다 |

- **토큰**: 회차마다 새로 만든다. 이전 회차의 남은 신호가 섞이지 않는다.
- **소비가 원자적이어야 한다**: 동시에 여러 요청이 같은 지점에 와도 붙잡히는 것은 하나다. 키 확인과 삭제도 한 스크립트 안에서 한다. 처음 구현은 `GETDEL`로 가져갔다가 키가 다르면 되돌려 놓았는데, 그 사이에 원래 대상이 오면 장전을 못 보고 지나간다. Twenty처럼 읽기 요청 하나가 여러 엔티티를 동시에 재충전하는 앱에서 생길 수 있는 틈이다.
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

결과는 사람이 읽는 회차별 출력과 요약, 회차별 JSON(`--out`), JUnit XML(`--junit`, 회차 하나가 테스트 케이스 하나)로 낸다. JUnit에서 `NOT_REACHED`·`ERROR` 회차는 통과도 실패도 아니므로 `skipped`로 적는다.

## 6. 실행 환경

실행기 바이너리도 실험 환경과 같은 격리 네트워크 안에서 돈다 (`lab/twenty/isolation.sh`). 빌드는 로컬 cargo 캐시만으로(`--offline`) 한다. 새 크레이트가 필요해지면 네트워크 사용 허락을 먼저 받는다.

## 7. 첫 대상: Twenty (2026-10-16)

`lab/twenty/scenarios/stale-set.yaml`을 `lab/twenty/verify.sh`로 돌렸다. 서버 두 대, 실행기 바이너리 모두 격리 네임스페이스 안에서 돈다.

- **훅 연결**: `refill-delay.patch` 안의 자체 동기화 지점(`cw:sync:*`, redis 클라이언트 직접 생성)을 `cawit-hook` 호출로 바꿨다. 앱 코드 변경은 4줄(주석 2, `require` 1, `syncPoint` 호출 1)이다. 지점은 `refill:after-read`, 키는 `<엔티티 종류>:<id>`. 실행기는 `workspaceEntity:<id>`로 장전한다. 읽기 요청 하나가 사용자·워크스페이스 등 여러 엔티티를 재충전하므로 키 없이 장전하면 엉뚱한 재충전이 붙잡힐 수 있다.
- **훅 설치**: 네트워크 없이 `hooks/node`의 세 파일을 대상의 `node_modules/cawit-hook`에 복사한다(`npm install hooks/node`와 같은 결과).
- **GraphQL 오류**: GraphQL은 오류에도 HTTP 200을 준다. 요청 템플릿에 `error_field: errors`를 둬서 응답에 그 필드가 있으면 `ERROR`로 처리한다.

| 수정 (각 20회) | not_stale | refilled | 결과 | 종료 코드 |
|---|---|---|---|---|
| 없음 | 0/20 | 0/20 | FAIL 20 | 1 |
| 버전 비교 SET (`vcas`) | 0/20 | 0/20 | FAIL 20 | 1 |
| 리스 | 20/20 | 0/20 | FAIL 20 | 1 |
| 리스 + 거부 시 다시 읽기 | 20/20 | 20/20 | PASS 20 | 0 |

모든 회차가 훅에 도달했다(`NOT_REACHED` 0). 1단계 `sync.sh` 결과와 같다. 관문 중 Twenty 몫(수정 전 매번 실패, 올바른 수정 매번 통과, 틀린 수정 두 개 판별, 훅 10줄 이내)은 충족했고, 두 번째 Node/TS 대상이 남았다.

## 8. 두 번째 대상 선정 (2026-10-16, 제안)

관문은 "Twenty + Node/TS 하나 더"다. 두 번째 대상은 Twenty와 **최대한 다른 캐시 구조**여야 훅·시나리오 형식이 한 앱에 맞춰진 것이 아님을 보일 수 있다.

**제안: Directus** (Express + knex, PostgreSQL, `CACHE_STORE=redis`). 웹 소스 열람으로 확인한 경로(main 브랜치, 아직 고정 커밋 아님):

- 읽기: `api/src/middleware/cache.ts`가 GET 미스면 `next()`로 컨트롤러가 DB를 읽게 하고, `api/src/middleware/respond.ts`가 응답 본문을 `setCacheValue(cache, key, payload, ttl)`로 쓴다. **DB 읽기 → 캐시 쓰기 사이**가 훅 자리다.
- 무효화: `api/src/services/items.ts`의 create·update·delete가 트랜잭션이 끝난 **뒤** `cache.clear()`(네임스페이스 전체 퍼지, `CACHE_AUTO_PURGE`)를 부른다. 순서는 맞지만 stale-set 경쟁에는 열려 있다: 리더가 옛 행을 읽음 → 라이터 커밋·퍼지 → 리더가 옛 응답을 `CACHE_TTL` 동안 넣어 둠.
- 키: `api/src/utils/get-cache-key.ts`가 버전·사용자·경로·쿼리의 해시로 만든다.

| Twenty와 다른 점 | Twenty | Directus |
|---|---|---|
| 프레임워크 / ORM | NestJS / TypeORM | Express / knex |
| 캐시 단위 | 엔티티 (`...:workspace:<id>:data`) | HTTP 응답 본문 (해시 키) |
| 무효화 | 키 단위 DEL | 네임스페이스 전체 퍼지 |
| 프로세스 안 캐시 | L1 + 메모이저 | 없음 (확인 필요) |

**코어에 생기는 요구**: 키가 해시라 `state.cache_key`로 직접 읽기 어렵다. "앱을 거쳐 읽은 값이 DB와 같은가"(응답 헤더로 캐시 적중 여부 확인)를 보는 판정 검사를 레지스트리에 추가한다. 이것이 판정 확장 구조의 첫 실사용이다.

**라이선스**: Directus는 MSCL-1.0(내부 사용·연구 허용, 4년 뒤 GPL-3.0). 우리는 소스를 재배포하지 않고 고정 커밋에 적용할 테스트 패치만 둔다.

**기각·보류한 후보**

| 후보 | 이유 |
|---|---|
| Medusa v2.11 캐싱 모듈 (MIT) | 태그 기반, 이벤트로 무효화. 구조가 흥미롭고 커밋 전·후 순서 문제 시나리오(3단계)에 더 맞다. 셋업(스토어 시드, 관리자)이 무겁다. 세 번째 후보로 보류 |
| n8n | Redis를 주로 큐(Bull)로 쓴다. 우리가 원하는 DB 행 cache-aside 경로를 확인하지 못했다 |
| Outline | 문서 cache-aside 경로를 확인하지 못했다 |
| cache-manager 예제 앱 | 직접 만든 앱은 "실제 앱" 근거가 약하다. 라이브러리 단위 시험은 별도로 둔다 |

**확인 전 가정**: `shouldClearCache` 조건, Redis 저장소의 `clear()`가 여러 인스턴스에서 같은 네임스페이스를 지우는지, 프로세스 안 캐시 유무. 소스를 받은 뒤 확인한다.

## 9. 사각지대

- 훅은 **알려 준 지점**에서만 순서를 통제한다. 재충전 경로가 여러 개면 지점도 여러 개 걸어야 한다. 빠뜨린 경로는 시험되지 않는다.
- 시나리오의 판정은 "그 키의 최종 상태"다. 프로세스 안 캐시(L1, 메모이저)가 낡은 값을 응답하는 문제는 "다시 채워지는가" 검사로 간접적으로만 잡는다. 1단계에서 리스만 넣은 수정이 이렇게 잡혔다.
- 서버 두 대를 띄우는 시나리오는 앱이 수평 확장을 지원해야 한다. 서버 한 대 안에서의 경쟁 시나리오는 3단계에서 다룬다.
