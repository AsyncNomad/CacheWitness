# 시작하기

이 페이지는 PostgreSQL 앞에 Redis 캐시를 둔 기존 Node 애플리케이션에, 대표적인 stale-set 경쟁을 강제로 일으키는 테스트를 추가합니다. 15분 정도 걸립니다.

필요한 것:

- 애플리케이션을 PostgreSQL 하나와 Redis 하나를 공유하는 **인스턴스 두 개**(포트 두 개)로 로컬에서 띄울 수 있을 것 (인스턴스 하나도 됩니다. [시나리오 작성](guide/writing-scenarios.md#5-인스턴스-하나-또는-둘) 참고)
- 캐시된 항목을 읽는 엔드포인트와 그 항목을 바꾸는 엔드포인트
- Rust(실행기 설치용)와 Node 18 이상

## 1. 설치

```bash
cargo install cawit                 # 실행기: 단일 바이너리
npm install --save-dev cawit-hook   # 훅: 의존성 없음
```

> v0.1이 배포되기 전까지는 저장소에서 실행기를 빌드하고(`cargo build --release`, 바이너리는 `target/release/cawit`) 훅은 `hooks/node`에서 설치합니다.

## 2. 재충전 경로를 찾아 훅 넣기

캐시 미스 때 실행되는 코드를 찾습니다. DB를 읽고 캐시에 씁니다. **읽기와 쓰기 사이**에 호출 하나를 넣습니다.

```ts
import * as cawit from "cawit-hook";

async function getArticle(id: string) {
  const key = `article:${id}`;
  const cached = await redis.get(key);
  if (cached) return JSON.parse(cached);

  const row = await db.query("select * from articles where id = $1", [id]);
  await cawit.syncPoint("refill:after-read", `/articles/${id}`);   // ← 훅
  await redis.set(key, JSON.stringify(row), "EX", 300);
  return row;
}
```

첫 번째 인자는 지점 이름이고, 두 번째는 시나리오가 특정 재충전 하나만 붙잡을 때 쓰는 키입니다. 어떤 문자열이든 되며, 여기서는 시나리오에 적기 쉬운 요청 경로를 썼습니다.

`CAWIT_SYNC_URL`이 없으면 이 호출은 아무 일도(I/O도) 하지 않습니다. 캐시 라이브러리, 번들러, 운영 환경 안전장치는 [Node 훅](guide/node-hook.md)을 보십시오.

## 3. 훅을 켜고 애플리케이션 띄우기

```bash
export CAWIT_SYNC_URL=redis://127.0.0.1:6379   # 실행기가 쓸 것과 같은 Redis
PORT=3000 npm start &
PORT=3001 npm start &
```

## 4. 경쟁을 적기

`stale-set.yaml`을 만듭니다.

```yaml
name: article-stale-set
runs: 20
apps:
  a: http://127.0.0.1:3000
  b: http://127.0.0.1:3001
redis: redis://127.0.0.1:6379
postgres: "host=127.0.0.1 user=postgres password=postgres dbname=app"

requests:
  update:
    method: PATCH
    path: /articles/1
    json: { title: "${value}" }
  read:
    method: GET
    path: /articles/1

state:
  db_query: "select title from articles where id = 1"
  cache_key: "article:1"
  cache_field: title

pattern:
  stale-set:
    reader: a
    writer: b
    write: update
    read: read
    key: /articles/1
```

- `requests`는 시나리오가 보낼 수 있는 HTTP 호출입니다. 쓰기 요청은 새 값이 들어갈 자리에 `${value}`를 둬야 하며, 회차마다 고유한 값을 씁니다.
- `state`는 무엇을 비교할지 정합니다. `article:1`에 저장된 JSON의 `title` 필드를 DB의 `title`과 비교합니다. 캐시 키가 해시이거나 값이 압축돼 있으면 대신 `app_view`로 애플리케이션을 거쳐 읽습니다([시나리오 형식](reference/scenario-format.md#상태)).
- `pattern: stale-set`은 경쟁을 강제하는 단계들로 펼쳐집니다([경쟁 패턴](guide/patterns.md#stale-set)).

무엇이 실행될지 정확히 봅니다.

```bash
cawit verify stale-set.yaml --show-steps
```

## 5. 실행

```bash
cawit verify stale-set.yaml
```

수정이 없으면 모든 회차가 실패해야 합니다.

```text
run   1  FAIL
          FAIL not_stale (cache_not_stale): cache=…-1-a db=…-1-b
          FAIL refilled (cache_equals_db): cache=…-1-a db=…-1-b
...
article-stale-set: 20 runs  pass 0  fail 20  not_reached 0  error 0
  not_stale            passed 0/20
  refilled             passed 0/20
```

종료 코드는 `1`입니다. 원할 때 재현되는 경쟁입니다.

회차가 대신 `NOT_REACHED`로 나오면 멈춰야 할 요청이 훅에 닿지 않은 것입니다. [문제 해결](guide/troubleshooting.md#not_reached)을 보십시오.

## 6. 고치고, 고쳐졌음을 증명하기

수정을 적용하고(예: 리스, 세대 번호 확인. [경쟁 패턴](guide/patterns.md#통과하는-수정과-통과하지-못하는-수정) 참고) 애플리케이션을 다시 띄운 뒤 같은 시나리오를 돌립니다. 올바른 수정은 모든 회차를 통과하고 `0`으로 끝납니다. 시나리오를 저장소에 두고 CI에서 돌립니다([CI에서 실행](guide/ci.md)).

## 다음 단계

- [핵심 개념](guide/concepts.md): 동기화 지점, 회차, 조작 확인, 판정
- [경쟁 패턴](guide/patterns.md): 내장 경쟁 세 가지와, 그것을 견디는 수정
- [사례 연구](case-studies/directus.md): Twenty와 Directus에 대한 완전한 시나리오
