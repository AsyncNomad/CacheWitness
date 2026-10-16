# Node 훅

`cawit-hook`은 [훅 프로토콜](../reference/hook-protocol.md)의 Node 구현입니다. 의존성이 없고 TypeScript 타입을 함께 제공합니다.

```bash
npm install --save-dev cawit-hook
```

```ts
import * as cawit from "cawit-hook";          // ESM / TypeScript
// const cawit = require("cawit-hook");       // CommonJS
```

## API

### `syncPoint(point: string, key?: string): Promise<void>`

시나리오가 호출을 붙잡거나 실패시킬 수 있는 이름 붙은 지점입니다.

- `point`: 시나리오가 쓰는 동기화 지점 이름(`refill:after-read`, `invalidate:before`, `write:before-commit`, 또는 직접 정한 이름)
- `key`: 선택. 시나리오는 지점을 특정 키에 대해서만 장전할 수 있고, 다른 키의 호출은 장전을 건드리지 않고 지나갑니다. 기본값은 `*`

동작:

| 상태 | 호출이 하는 일 |
|---|---|
| 훅 꺼짐 (`CAWIT_SYNC_URL` 없음, 또는 운영 환경 안전장치) | 바로 반환, I/O 없음 |
| 켜짐, 장전 안 됨 (또는 다른 키로 장전) | Redis 왕복 한 번 뒤 반환 |
| `hold`로 장전 | 실행기에 알린 뒤 해제될 때까지(또는 `CAWIT_SYNC_TIMEOUT_S`까지) 대기 |
| `fail`로 장전 | 실행기에 알린 뒤 `CawitInjectedError`를 던짐 (한 번) |

### `enabled: boolean`

이 프로세스에서 훅이 켜져 있는지 (로드할 때 한 번 읽음).

### `CawitInjectedError`

`fail`로 장전된 지점을 가져갔을 때 던지는 오류.

### `close(): void`

훅의 공유 Redis 연결을 닫습니다 (테스트, 정상 종료용).

## 환경 변수

| 변수 | 기본값 | 의미 |
|---|---|---|
| `CAWIT_SYNC_URL` | 없음 | 실행기도 쓰는 `redis://[[user]:password@]host[:port][/db]`, TLS는 `rediss://`. 없으면 훅이 꺼짐 |
| `CAWIT_SYNC_TIMEOUT_S` | `30` | 붙잡힌 호출이 해제를 기다리는 시간(소수 가능). 실행기는 시간 초과된 붙잡기를 `NOT_REACHED`로 봄 |
| `CAWIT_ALLOW_PRODUCTION` | 없음 | `NODE_ENV=production`이면 이 값이 `1`이 아닌 한 훅이 꺼진 채로 있음 |
| `CAWIT_SYNC_TLS_INSECURE` | 없음 | `1`: `rediss://` 서버의 인증서를 검증하지 않음 (시험 서버 전용) |

`rediss://`에서 훅은 Node의 CA 목록과 `NODE_EXTRA_CA_CERTS`를 신뢰합니다. URL에 비밀번호가 있으면 여는 연결마다 `AUTH`를 보내고, `AUTH`가 실패하면 호출이 이유와 함께 예외를 던집니다(비밀번호는 출력하지 않음).

## 호출을 어디에 넣을까

**재충전 (stale-set).** DB 읽기와 캐시 쓰기 사이:

```ts
const row = await repo.findOneBy({ id });
await cawit.syncPoint("refill:after-read", `article:${id}`);
await cache.set(`article:${id}`, row, ttl);
```

`cache.wrap(key, loader)` 같은 읽기 위임 도우미를 쓴다면, 로더의 끝, DB 읽기 뒤에 넣습니다.

```ts
return cache.wrap(`article:${id}`, async () => {
  const row = await repo.findOneBy({ id });
  await cawit.syncPoint("refill:after-read", `article:${id}`);
  return row;
});
```

**무효화 (failed-invalidation).** 항목을 지우는 호출 바로 앞:

```ts
await cawit.syncPoint("invalidate:before", `article:${id}`);
await cache.del(`article:${id}`);
```

**커밋 전 (invalidate-before-commit).** 트랜잭션 안의 마지막 문장:

```ts
await db.transaction(async (trx) => {
  await trx("articles").update({ title }).where({ id });
  await cache.del(`article:${id}`);              // 시험할 버그
  await cawit.syncPoint("write:before-commit", `article:${id}`);
});
```

## 연결

훅은 애플리케이션의 Redis 연결을 절대 쓰지 않습니다. 막히지 않는 명령은 훅 연결 하나를 같이 쓰고, 붙잡힌 호출은 저마다 자기 연결에서 기다립니다. 그래서 붙잡힌 요청이 같은 프로세스의 다른 요청이나 다른 동기화 지점을 늦추지 않습니다. 훅 연결은 켜진 상태에서 처음 호출될 때 엽니다.

## 번들러

빌드가 의존성을 묶는다면(tsdown, esbuild, webpack, ncc) `cawit-hook`을 외부 의존성으로 지정하거나, 업그레이드 뒤 다시 빌드하십시오. 빌드 안에 남은 옛 훅은 옛 프로토콜을 써서 장전을 가져가지 못하고, 모든 회차가 `NOT_REACHED`가 됩니다. [Directus 사례 연구](../case-studies/directus.md#배운-것)에서 실제로 겪었습니다.

## 운영 환경 안전성

- `CAWIT_SYNC_URL`이 없으면 호출은 아무 일도 하기 전에 돌아옵니다.
- `NODE_ENV=production`이면 `CAWIT_SYNC_URL`이 있어도 `CAWIT_ALLOW_PRODUCTION=1`이 아닌 한 훅이 꺼져 있습니다. 이 변수는 `NODE_ENV=production`으로 도는 시험 환경(스테이징 같은 E2E)에서만 주십시오.
- 켜져 있으면, 장전되지 않은 지점도 호출마다 Redis 왕복 한 번의 비용이 듭니다. 그 지연이 중요한 곳에서는 훅을 켜지 마십시오.

## 다른 언어

프로토콜은 Redis 명령 몇 개입니다([훅 프로토콜](../reference/hook-protocol.md)). 기존 Redis 클라이언트 라이브러리를 쓰면 다른 언어의 훅은 수십 줄이면 됩니다(Node 훅은 의존성을 없애려고 RESP 파서를 직접 들고 있어 약 150줄입니다). Python을 계획하고 있습니다.
