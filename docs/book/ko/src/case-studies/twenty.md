# 사례 연구: Twenty CRM

[Twenty](https://github.com/twentyhq/twenty)는 오픈소스 CRM입니다(NestJS, TypeORM, PostgreSQL, Redis). 인증된 모든 요청이 읽는 워크스페이스 레코드를 캐시하는 핵심 엔티티 캐시를 시험했습니다.

모든 실행은 로컬에서, 애플리케이션 실험 환경을 네트워크와 격리한 채 돌렸습니다. 고정 커밋: `c3ba1401`. 실험 파일: [`lab/twenty/`](https://github.com/AsyncNomad/CacheWitness/tree/main/lab/twenty).

## 캐시 구조

- 미스가 나면 `CoreEntityCacheService.recomputeFromProvider`가 PostgreSQL에서 엔티티를 읽어 Redis에 조건 없이 씁니다(`MSET`).
- 무효화는 쓰기 뒤에 항목을 지웁니다.
- Redis 앞에 프로세스 안 계층이 두 개 있습니다. 100ms마다 다시 확인하는 로컬 캐시와, 해결된 값을 10초 동안 붙잡는 메모이저입니다.

## 훅

캐시 서비스에 네 줄(주석, import, 그리고 아래 호출)을 넣었습니다.

```ts
await cawit.syncPoint('refill:after-read', `${cacheKeyName}:${entityId}`);
```

제공자의 DB 읽기 바로 뒤입니다. 요청 하나가 여러 엔티티(사용자, 워크스페이스, API 키)를 다시 채우고 워크스페이스 재충전만 붙잡아야 하므로, 시나리오는 `workspaceEntity:<워크스페이스 id>`로 장전합니다.

## 시나리오

[`lab/twenty/scenarios/stale-set.yaml`](https://github.com/AsyncNomad/CacheWitness/blob/main/lab/twenty/scenarios/stale-set.yaml): 서버 두 대, 워크스페이스 이름을 바꾸는 GraphQL 뮤테이션, 그것을 읽는 쿼리, 그리고 캐시된 `displayName`과 DB를 비교하는 `state`. GraphQL 오류는 상태 200으로 오므로 두 요청 모두 `error_field: errors`를 둡니다.

## 결과 (각 20회, 모든 회차가 훅에 도달)

| 수정 후보 | not_stale | refilled | 결과 |
|---|---|---|---|
| 없음 | 0/20 | 0/20 | FAIL |
| 버전 비교 `SET` (캐시된 사본이 더 새롭지 않을 때만 씀) | 0/20 | 0/20 | FAIL |
| 리스 (무효화가 리스를 지우고, 리스가 남아 있을 때만 씀) | 20/20 | 0/20 | FAIL |
| 리스 + 리스를 잃으면 다시 읽기 | 20/20 | 20/20 | PASS |

## 배운 것

- **버전 비교 `SET`은 여기서 아무 효과가 없습니다.** 새 쓰기의 `DEL` 뒤에 캐시는 비어 있어 비교할 더 새로운 사본이 없으므로, 옛 값이 그대로 쓰입니다.
- **리스만 넣으면 맞아 보이지만 캐시가 망가집니다.** 낡은 쓰기는 막지만, 리스를 잃은 요청이 읽어 둔 값을 돌려주고, Redis가 빈 채로 메모이저가 그 값을 10초 동안 계속 응답합니다. `refilled` 판정만 이것을 잡습니다.
- **자연 발생 빈도는 낮습니다.** 순서를 강제하지 않으면, 한 사람이 1~3초 간격으로 저장할 때 477번 중 낡은 값이 없었습니다(95% 상한 0.8%). 동시 쓰기는 잠깐의 낡은 읽기(쓰기의 0.26~0.93%)를 만들었지만 최대 43ms 뒤 사라졌고, 쓰기 몰아치기가 끝난 뒤까지 남은 낡은 값은 없었습니다(약 1,100번 중 0). 재충전을 일부러 50ms 늦췄을 때만 몰아치기 뒤에 낡은 값이 남았고, 같은 조건에서 한 실행은 6.5%, 다른 실행은 0.17%였습니다. 실재하는 경쟁이지만 급한 버그로 보고할 정도는 아닙니다.
