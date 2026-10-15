# 1단계 결과 기록: 실제 앱 적합성

1단계의 목표는 가장 큰 리스크인 **실제 앱의 매핑 부담**을 싸게 확인하는 것이다 (PLAN 13장). 계획서의 통과 조건은 "오픈소스 앱 3개 중 2개 이상이 30줄 이내로 매핑되고, 테스트가 커밋하거나 커밋하도록 바꿀 수 있을 것"이다.

## 1. 코드 조사: PostgreSQL + Redis 캐시를 쓰는 오픈소스 8개 (2026-10-15)

각 저장소의 기본 브랜치를 얕게 클론해서 캐시 계층의 코드를 읽었다.

| 앱 | 언어 | 캐시 계층 | Redis에 들어가는 값 | 무엇을 캐시하나 | 키 형식 | v0(JSON)로 읽을 수 있나 |
|---|---|---|---|---|---|---|
| Twenty | TS (NestJS) | `cache-manager-redis-yet`, 자체 `CoreEntityCacheService` | JSON | **행 캐시**: user, workspace, userWorkspace (+ 버전 표시용 `:hash` 키, 부정 캐시 표식) | `[테스트 접두사:]engine:core-entity:<엔티티>:<uuid>:data` | **예** |
| n8n | TS | ioredis 기반 cache-manager | JSON | **행 캐시**: 웹훅 (`webhook:<method>-<path>`), 그 밖에 설정·토큰 | `{n8n:cache}:` 해시태그 접두사 + 키 | 예. 단 키가 복합 키(`method`, `path`)이고 구분자 `-`가 경로에도 나와 역해석이 모호. 기본 설정은 메모리 캐시이고 Redis는 큐 모드에서만 쓴다 |
| Medusa | TS | 자체 `caching-redis` 모듈 | HSET `data` 필드에 JSON, 2KB 넘으면 **gzip** | **쿼리 결과 캐시**, 태그로 무효화 | 해시된 키 | 행 매핑 불가 |
| PostHog | Python (Django) | django-redis + zstd 압축기 | **pickle(JSON 문자열)**, 크면 **zstd** | 행 캐시: team (`team_token:<api_token>`), 그 밖에 집계·쿼리 캐시 다수 (호출 지점 557곳) | `posthog:1:<키>` | 아니오 (pickle·zstd 해제가 필요). 키가 기본 키가 아니라 유일 컬럼(`api_token`). Redis 레플리카 읽기 사용 |
| Paperless-ngx | Python (Django) | Django `RedisCache` + django-cachalot | pickle | 문서 메타데이터·분류기 결과 등 파생 값, cachalot 쿼리 캐시 | `<접두사>:1:<키>` | 아니오. 행 매핑도 거의 불가 |
| Saleor | Python (Django) | `django_cache_url` | pickle | 소수 (호출 지점 28곳) | `:1:<키>` | 아니오 |
| Mastodon | Ruby (Rails 8.1) | `redis_cache_store` | Rails 캐시 형식(Marshal 기반) | 목록·파생 값 대부분 (차단 ID 목록, 필터, 추천) | 자유 형식 | 아니오 |
| Discourse | Ruby | 자체 `Cache` 클래스 | Ruby 직렬화 | 목록·파생 값 | 네임스페이스 접두사 | 아니오 |

**관찰**

- **JSON을 쓰는 앱은 Node 계열뿐이다.** Django 계열은 pickle, Rails 계열은 Marshal이 기본이다. PLAN 9.3의 우려가 그대로 맞았다.
- **행 캐시는 생각보다 드물다.** 8개 중 행 단위 cache-aside가 핵심인 앱은 Twenty, n8n, PostHog(team) 정도다. 나머지는 쿼리 결과·목록·파생 값 캐시가 중심이라 "테이블 1:1 매핑"이 성립하지 않는다.
- **값이 여러 겹으로 싸여 있다.** PostHog는 JSON 문자열을 pickle하고, 크면 zstd로 압축한다. Medusa는 2KB 이상이면 gzip을 쓴다. 디코더는 하나가 아니라 **연쇄**(zstd → pickle → JSON)로 설계해야 한다.
- **키가 기본 키가 아닌 경우가 있다.** PostHog는 `api_token`(유일 컬럼), n8n은 복합 키다. 매핑에 "조회 컬럼" 개념이 필요하다.
- **접두사가 환경마다 다르다.** 테스트 전용 접두사(Twenty `integration-tests:`), 해시태그(n8n `{...}`), Django 버전 접두사(`:1:`).

---

## 2. Twenty를 실제로 돌려 본 결과

조사에서 v0와 가장 잘 맞는 Twenty를 골라, 통합 테스트를 실제 PostgreSQL 16과 Redis 7.2에 붙여 돌렸다.

- 환경: `lab/twenty/compose.yaml`, 기록: `lab/twenty/record.sh`, 분석: `lab/twenty/analyze.py`
- Redis는 `MONITOR`로 모든 명령을 기록했다. 프로토타입은 MGET·MSET·다중 키 DEL을 해석하지 못하므로, 이 측정에는 MONITOR가 맞다.
- PostgreSQL은 DB 초기화 뒤에 만든 test_decoding 슬롯으로 커밋을 기록했다.

### 2.1 테스트 환경의 수명주기

| 항목 | 실제 |
|---|---|
| DB 초기화 | `DROP SCHEMA ... CASCADE` 후 마이그레이션 (TRUNCATE도 DB 재생성도 아니다) |
| 테스트 데이터 | 커밋한다 (트랜잭션 롤백 방식 아님). check 모드의 전제를 만족한다 |
| 스키마 | 워크스페이스마다 스키마를 실행 중에 새로 만든다 (`workspace_<id>`) |
| 병렬성 | `maxConcurrency: 1`, `--runInBand` |
| Redis | **캐시와 BullMQ 큐가 같은 Redis를 쓴다.** 명령의 대부분이 큐 트래픽이다 |
| 키 접두사 | 테스트에서는 `integration-tests:`가 앞에 붙는다 |

**계획에 주는 의미**

- 스키마 단위 DROP은 PLAN 9.2의 "DB 재생성" 처리와 다르다. 테이블 단위 publication은 스키마가 지워지면 함께 사라지므로, attach-on-ready는 **스키마 재생성**도 감지해야 한다.
- 캐시와 큐가 같은 Redis에 있으면 keyspace 알림이 큐 트래픽까지 모두 발행된다. 구독 패턴을 매핑된 키로 좁혀도 서버의 발행 비용은 그대로다 (PLAN 17장의 비용 측정 항목).

### 2.2 v0 명령 범위와의 차이

엔티티 캐시 키에 실제로 쓰인 명령:

| 명령 | 용도 | PLAN 9.1 v0 범위 | 프로토타입 |
|---|---|---|---|
| `MGET` | `:data`와 `:hash`를 한 번에 읽기 | 포함 | **미지원** |
| `SET ... PX` | 캐시 채우기 | 포함 | 지원 |
| `DEL k1 k2` | `:data`와 `:hash`를 한 번에 무효화 | 포함 | **첫 키만 기록** |
| `EVAL` (내부에서 `SET`) | 여러 키를 TTL과 함께 원자적으로 쓰기 | 키 선언 + keyspace 알림 | **미지원** |
| `GET` | `:hash`만 읽어 로컬 캐시 검증 | 포함 | 지원 |

- **MONITOR는 Lua 스크립트 안의 명령도 `[0 lua]`로 기록한다.** 스크립트 내부 쓰기를 관측하는 또 다른 방법이다. 다만 MONITOR는 서버 성능에 영향이 커서 CI 전용이다.

### 2.3 매핑 부담

캐시된 값의 필드는 **모두 DB 컬럼 이름과 같고**, 컬럼의 부분집합이다.

| 엔티티 | 캐시 필드 | 컬럼 | 캐시에 없는 컬럼 |
|---|---|---|---|
| workspace | 39 | 40 | `deletedAt` |
| user | 11 | 13 | `deletedAt`, `passwordHash` |
| userWorkspace | 8 | 8 | — |

그래서 필드 이름 매핑 없이 엔티티마다 4~5줄로 매핑된다.

```yaml
mappings:
  - key: "{prefix}engine:core-entity:user:{id}:data"
    table: core."user"
    pk: [id]
    value: { format: json }
    soft_delete: deletedAt
  - key: "{prefix}engine:core-entity:workspace:{id}:data"
    table: core.workspace
    pk: [id]
    value: { format: json }
    soft_delete: deletedAt
  - key: "{prefix}engine:core-entity:user-workspace:{id}:data"
    table: core."userWorkspace"
    pk: [id]
    value: { format: json }
    negative: { value: "\"__CORE_ENTITY_NOT_FOUND__\"" }
# ":hash" 키는 랜덤 UUID 버전 표식이다. 행과 대응하지 않으므로 미매핑으로 둔다.
```

**약 17줄로 30줄 기준을 통과한다.** 다만 두 가지는 현재 설계에 없다.

- **소프트 삭제**: `deletedAt`은 캐시되지 않는다. 그래서 행이 소프트 삭제돼도 겹치는 필드끼리 비교하면 "최신"으로 보인다. `soft_delete` 컬럼을 tombstone으로 다루는 설정이 필요하다.
- **버전 표식 키**: `:hash`처럼 행이 아니라 캐시 자신의 버전을 나타내는 키가 있다. 미매핑으로 나열하되, "버전 표식"으로 분류하면 보고서의 미매핑 목록이 덜 시끄럽다.

### 2.4 첫 발견: 삭제된 userWorkspace가 캐시에 7일 남는다

`graphql/suites/user*` 스위트 11개를 돌린 기록에서, 엔티티 행이 캐시된 상태에서 바뀐 경우가 13건이었다. 12건은 바로 무효화됐고, **1건은 끝까지 무효화되지 않았다.**

- `core."userWorkspace"` 행이 **DELETE**됐는데, 캐시 항목 `...:user-workspace:<id>:data`가 그대로 남았다. TTL은 `PX 604800000`(7일)이다.
- 코드 확인: `UserWorkspaceService.deleteUserWorkspace`는 행을 지우지만 엔티티 캐시를 무효화하지 않는다. 호출 지점 세 곳 중 `workspace.service.ts`만 호출 뒤에 `invalidate('userWorkspaceEntity', ...)`를 부르고, **`workspace-member-delete-one.post-query.hook.ts`와 `user.service.ts`는 부르지 않는다.**
- 영향 확인: 이 캐시를 읽는 곳은 JWT 인증 전략(`resolveUserContext`) 한 곳뿐이다. 그런데 인증 전략은 그 뒤에 워크스페이스 멤버 맵을 다시 확인하고, 이 맵은 두 경로 모두에서 다시 계산된다. **따라서 제거된 멤버가 인증을 통과하지는 않는다.** 예외는 워크스페이스가 생성 중 상태일 때다(그 확인을 건너뛴다).

**분류**: 실제 캐시 불일치(삭제된 행이 최대 7일 캐시됨)이지만, 이중 확인 때문에 사용자 피해는 드러나지 않는다. 검토에서 지적한 "탐지는 됐지만 버그로 인정받기 어려운 경우"(PLAN 15장)의 실제 사례다. upstream에 보고한다면 보안 문제가 아니라 일관성 문제로, 이중 확인이 있다는 점을 함께 적어야 한다.

(Twenty 커밋 `c3ba1401`, 2026-10-14 기준)

---

### 2.5 전체 통합 테스트 기록

같은 방식으로 전체 통합 테스트를 돌렸다. jest 프로세스가 스위트 **432개**를 돈 뒤 8GB 힙 한도를 넘겨 종료됐으므로, 아래는 그 432개 분량이다.

| 항목 | 값 |
|---|---|
| Redis 명령 | 약 685만 건. 대부분 BullMQ 큐(`ZCARD`, `LLEN`, `LINDEX`, `EVALSHA` 66만 건). **엔티티 캐시 관련은 9,116줄(0.13%)** |
| MONITOR 기록 크기 | 18GB |
| 엔티티 행 변경 (WAL) | workspace UPDATE 1,621, user UPDATE 54, INSERT·DELETE 수십 건 |
| 캐시된 상태에서 행이 바뀐 경우 | **1,528건** |
| 그중 무효화된 경우 | 1,527건. 커밋 → 무효화 중앙값 **1.7ms**, p90 6.5ms |
| 1초 넘게 걸린 경우 | 20건. **17건은 시계 이동 때문에 생긴 가짜**(2.6), 남은 3건은 아래 표 |
| 끝까지 무효화되지 않은 경우 | 1건 (아래 표) |

| 경우 | 관측 | 판정 |
|---|---|---|
| workspace `activationStatus` → SUSPENDED, 36.5초 뒤 무효화 | 37ms 뒤 다시 ACTIVE로 돌아감 | **테스트 픽스처**가 SQL로 직접 바꾼 것(`failing-sign-up-in-non-active-workspace`). 앱을 거치지 않았고, 낡은 구간도 허용치(500ms)보다 짧다 |
| user DELETE, 무효화 없음 | 같은 트랜잭션에서 appToken·userSession도 삭제 | **테스트 픽스처**의 `DELETE FROM core."user"` (여러 스펙에 있음) |
| workspace `logoFileId` → null, 8.3초 뒤 무효화 | `updateWorkspace(logo: null)` 뮤테이션 경로. 코드는 UPDATE 직후 `invalidate`를 부르는데, 그 사이 Redis 트래픽은 초당 약 8천 건으로 계속됐다 | **판단 불가.** 앱이 늦었는지, 다른 경로의 커밋인지 프록시·WAL만으로는 가릴 수 없다. 앱 쪽 증거(요청 ID, 호출 시각)가 필요하다 |

2.4의 userWorkspace DELETE(앱 경로의 무효화 누락)는 이 실행에서는 다시 나오지 않았다. 해당 시점에 캐시돼 있지 않았거나, 그 스위트가 OOM 전에 돌지 않았기 때문이다.

### 2.6 측정 함정: Redis MONITOR의 타임스탬프도 벽시계다

처음 분석에서는 "1초 넘게 늦은 무효화"가 20건 나왔다. 그런데 MONITOR 기록은 Redis가 실행한 순서인데도 타임스탬프가 +2,632ms에서 +1.7ms로 **거꾸로** 갔다. Redis와 PostgreSQL 모두 벽시계를 쓰므로, WSL2의 약 −2.8초 이동이 두 기록에 그대로 들어간다(이 실행 중 104회 감지). 실행 순서로 다시 보면, 해당 커밋 직후 1.7ms 만에 정상적으로 무효화됐다.

분석기는 두 기록에서 시계가 거꾸로 가는 지점을 찾고, 그 근처(±3.5초)에 걸린 판정을 따로 표시하도록 고쳤다. **0단계(3.1)와 같은 교훈이 다른 형태로 다시 나왔다.** 서로 다른 프로세스의 벽시계 타임스탬프를 비교하는 순간, 이동 근처의 판정은 믿을 수 없다.

---

## 3. 통과 조건 판정

| 앱 (생태계) | 30줄 이내 매핑 | 테스트가 커밋하는가 | 판정 |
|---|---|---|---|
| Twenty (Node/TS) | **예** (약 17줄) | 예 | 통과. 단 아래 4장의 기능이 필요 |
| PostHog (Django) | 아니오. pickle + zstd 디코더 필요, 키가 유일 컬럼 | (실행하지 않음. ClickHouse·Kafka 등 의존성이 무겁다) | v0로는 불가 |
| Mastodon·Discourse (Rails) | 아니오. Marshal 기반 직렬화, 캐시 대부분이 목록·파생 값 | (실행하지 않음) | v0로는 불가 |

참고로 n8n(Node/TS)도 JSON이라 읽을 수 있지만, 키가 복합 키이고 Redis 캐시는 큐 모드에서만 쓴다.

**판정: 매핑 모델은 통과, 직렬화 포맷 때문에 불통과.** 계획서 13장의 분기에 따라 다음과 같이 정했다 (2026-10-15).

- **v0 범위는 "JSON 직렬화 생태계(주로 Node/TS)"로 명시한다.** 디코더는 `ValueDecoder` 확장 지점으로 남긴다.
- **단순 pickle 디코더는 보류한다.** 디코더로 판정 가능해지는 대상은 PostHog 하나뿐이다(Medusa는 값을 읽어도 쿼리 결과 캐시라 행·버전을 정할 수 없다). 그런데 PostHog의 테스트는 대부분 롤백 방식이다. 기본 테스트 클래스 `BaseTest`·`APIBaseTest`가 `TestCase`이고, 이를 쓰는 파일이 약 2,680개, 커밋하는 `TransactionTestCase` 계열은 약 230개다. check 모드 결과가 대부분 `INCONCLUSIVE`가 된다. (Redis 레플리카 읽기는 `REDIS_READER_URL` 기본값이 `None`이라 테스트에서는 문제되지 않는다. zstd 압축은 `USE_REDIS_COMPRESSION`이 켜져 있고 512바이트를 넘을 때만 적용된다.)
- **gzip·zstd 압축 해제는 저렴하지만 지금 이걸로 판정 가능해지는 대상이 없으므로** 필요해질 때 넣는다.
- 처음 권고("디코더 연쇄를 v0에 넣는다")는 Medusa를 판정 가능한 대상으로 잘못 셌다. 외부 검토에서 지적받아 고쳤다.

**check 모드의 수확이 적을 수 있다.** 잘 관리된 코드베이스(Twenty)에서는 캐시된 행 변경 1,528건 중 1,527건이 바로 무효화됐다. 결정적인 무효화 누락은 드물다. 프로젝트의 핵심 가설("교과서 패턴도 동시성 아래에서 낡은 값을 남긴다")은 아직 실제 앱에서 검증하지 않았으므로, 다음 작업은 Twenty에서의 race 검증이다(5장).

**OOM은 실행 방식 탓이다.** Twenty CI는 통합 테스트를 16개 샤드로 나누고 샤드마다 12GB 힙을 준다. 한 프로세스로 전체를 돌린 것이 원인이다. MONITOR는 별도 `redis-cli` 프로세스가 파일로 쓰므로 Node 힙과 무관하다.

---

## 4. 설계에 반영할 것

| 발견 | 반영 |
|---|---|
| 테스트 픽스처가 앱을 거치지 않고 DB를 직접 고친다 (Twenty의 여러 스펙) | **쓰기 출처 구분이 필요하다.** 그대로 두면 픽스처 쓰기가 "무효화 누락"으로 보고된다. **`log_line_prefix`의 `%x`로 WAL의 xid와 맞추는 방법은 쓸 수 없다**: 실험해 보니 자동 커밋 문장은 실행 전·후 로그 모두 `%x`가 0이었다(명시적 트랜잭션 안에서만 xid가 찍힌다). 남은 후보: 픽스처 연결에서 복제 origin 설정(`pg_replication_origin_session_setup`, superuser 필요, 미검증), `pg_logical_emit_message`로 픽스처 구간 표시(픽스처 코드 수정), 출처를 모르는 커밋을 `UNKNOWN_WRITER`로 분류. 2단계 설계에서 정한다 |
| 같은 호스트의 Redis와 PostgreSQL은 같은 벽시계를 읽는다 | **두 시계를 서로 비교해서는 시계 이동을 감지할 수 없다.** 75초 실측에서 두 시계가 함께 약 −2.95초씩 3번 이동했고, 둘의 차이는 내내 약 +70ms(측정 지연)로 일정했다. 도구 자체의 단조 시계와 비교해야 한다 (PLAN 9.5) |
| 캐시에 없는 컬럼으로 소프트 삭제한다 (`deletedAt`) | 매핑에 `soft_delete` 컬럼. 값이 생기면 tombstone으로 처리 |
| 부정 캐시 표식 값 (`"__CORE_ENTITY_NOT_FOUND__"`) | 매핑의 `negative` (PLAN 9.3에 이미 있음) |
| 행이 아니라 캐시 자신의 버전을 나타내는 키 (`:hash`) | 미매핑 중 "버전 표식"으로 분류해 보고서 소음을 줄인다 |
| 값이 여러 겹이다 (JSON → pickle → zstd, JSON → gzip) | `ValueDecoder`를 연쇄로 구성 |
| 키가 기본 키가 아니다 (유일 컬럼, 복합 키) | 매핑에 조회 컬럼(`lookup: [api_token]`) |
| 캐시와 큐가 같은 Redis를 쓴다 (명령의 99.9%가 큐) | 관측 계층은 매핑되지 않은 키를 파싱 직후 버린다. keyspace 알림 비용 측정에 큐 트래픽을 포함한다 |
| 실제 값이 크다 (워크스페이스 메타데이터 수 MB) | 값 전체를 보관하지 않고 필드 지문만 (PLAN 9.7 유지) |
| `DROP SCHEMA ... CASCADE` 초기화, 워크스페이스마다 스키마 생성 | attach-on-ready가 DB뿐 아니라 스키마 재생성도 감지 (PLAN 9.2) |
| MGET, 다중 키 DEL, EVAL 내부 SET | v0 명령 범위(PLAN 9.1)가 맞다. 프로토타입은 셋 다 처리하지 못했다 |
| MONITOR는 스크립트 내부 명령까지 실행 순서대로 보여준다 | CI 전용 보조 관측 수단으로 재검토할 가치가 있다. 응답을 볼 수 없다는 한계(PLAN 16장)는 그대로다 |

(Twenty 커밋 `c3ba1401`, 2026-10-14 기준. 기록은 `lab/out/twenty-*`에 있고 git에는 넣지 않는다.)

---

## 5. 다음 작업: Twenty race 검증

Twenty 엔티티 캐시의 미스 경로(`recomputeFromProvider`)는 DB를 읽은 뒤 `MSET`으로 `:hash`(랜덤 UUID)와 `:data`를 **조건 없이** 덮어쓴다. 리스나 버전 비교가 없다. `:hash`는 로컬 캐시 갱신용 표식이지 DB 버전이 아니다. 따라서 교과서적인 stale-set 경쟁이 가능한 구조다.

현실적인 상황으로 설계한다. 관리자가 워크스페이스 설정(이름, 메타데이터)을 바꾸는 동안, 다른 사용자들의 모든 인증 요청이 workspace 캐시를 읽는 상황이다. 억지 부하로 만든 경쟁은 버그로 인정받기 어렵기 때문이다 (PLAN 15장).
