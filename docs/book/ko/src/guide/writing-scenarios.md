# 시나리오 작성

"낡은 캐시 버그가 있는 것 같다"에서 출발해, 수정 전에는 실패하고 수정 후에는 통과하는 시나리오까지 가는 안내입니다.

## 1. 경쟁 고르기

| 증상 | 의심되는 경쟁 | 패턴 |
|---|---|---|
| 수정 뒤 리더가 가끔 TTL까지 옛 값을 계속 받음 | 수정 전에 시작한 재충전이 무효화 뒤에 끝남 | [`stale-set`](patterns.md#stale-set) |
| 수정 요청이 오류(또는 시간 초과)를 돌려줬는데 값은 바뀌었고, 캐시는 옛 값을 유지 | 커밋 뒤 무효화가 실패함 | [`failed-invalidation`](patterns.md#failed-invalidation) |
| 수정 직후, 그 전에 동시 리더가 없었는데도 리더가 옛 값을 받음 | 트랜잭션 안에서, 커밋 전에 캐시를 무효화함 | [`invalidate-before-commit`](patterns.md#invalidate-before-commit) |

## 2. 훅 위치 찾기

패턴마다 코드의 한 곳에 동기화 지점이 필요합니다.

| 패턴 | 지점 (기본 이름) | 위치 |
|---|---|---|
| `stale-set` | `refill:after-read` | 미스 경로에서 DB 읽기 뒤, 캐시 쓰기 전 |
| `failed-invalidation` | `invalidate:before` | 쓰기 경로의 캐시 무효화 바로 앞 |
| `invalidate-before-commit` | `write:before-commit` | 쓰기 트랜잭션의 끝, 커밋 전 |

코드에서 캐시 쓰기(`set`, `mset`, `setex`, 캐시 라이브러리의 `set`이나 `wrap`)와 무효화(`del`, `unlink`, `clear`, 태그·네임스페이스 퍼지)를 찾으십시오. 프레임워크는 이런 코드를 서비스나 미들웨어 한 곳에 모으는 경우가 많습니다. 사례 연구에서는 캐시 서비스 하나(Twenty)와 응답 미들웨어 하나(Directus)에 훅을 넣었습니다.

시나리오가 관심 있는 재충전만 붙잡을 수 있도록 **키**를 고르십시오. 읽기 요청 하나가 캐시 항목 여러 개를 다시 채우는 경우가 많습니다. 키 없이 장전하면 관계없는 재충전이 먼저 붙잡힐 수 있습니다. 요청 경로나 `<엔티티>:<id>`처럼 시나리오에 그대로 적을 수 있는 값을 쓰십시오.

## 3. 요청

쓰기용 요청 템플릿 하나와 읽기용 하나를 만듭니다. 쓰기는 쓰는 값 자리에 `${value}`(또는 패턴의 `value_var`로 정한 다른 변수)를 써야 회차마다 고유한 값을 씁니다.

```yaml
requests:
  update:
    method: PATCH
    path: /items/articles/1
    headers: { authorization: "Bearer ${env.API_TOKEN}" }
    json: { title: "${value}" }
  read:
    method: GET
    path: /items/articles/1
    headers: { authorization: "Bearer ${env.API_TOKEN}" }
```

GraphQL이라면 `error_field: errors`를 넣으십시오. GraphQL은 오류에도 상태 200을 주므로, 이것이 없으면 실패한 뮤테이션이 성공처럼 보입니다.

## 4. 상태: 무엇을 비교할지

`db_query`는 커밋된 값을 첫 행의 첫 열에 텍스트로 돌려줍니다.

그다음 캐시 쪽을 어떻게 읽을지 고릅니다.

- **`cache_key`**: Redis 키를 알고 값이 일반 텍스트나 JSON일 때. JSON의 최상위 필드 하나를 비교하려면 `cache_field`를 추가합니다.
- **`app_view`**: 키가 해시이거나, 값이 압축·직렬화돼 있거나, 중요한 것이 응답 자체일 때. 애플리케이션에 요청을 보내 응답 본문에서 JSON pointer로 값을 꺼냅니다. 애플리케이션이 캐시 적중을 헤더로 알려 준다면 `hit_header`를 추가하십시오. 그러면 `refilled` 판정이 적중까지 요구합니다.

## 5. 인스턴스 하나 또는 둘

시나리오는 경쟁 쓰기를 두 번째 인스턴스(`writer: b`)로 보낼 수도, 같은 인스턴스(`writer: a`)로 보낼 수도 있습니다. 둘 다 됩니다. 붙잡힌 호출은 저마다 자기 Redis 연결에서 기다리므로 같은 프로세스의 다른 요청을 막지 않습니다. 인스턴스 둘이 운영 환경에 더 가깝고 인스턴스 간 무효화까지 시험합니다.

## 6. 실행하고, 읽고, 다듬기

```bash
cawit verify scenario.yaml --show-steps   # 펼친 단계 확인
cawit verify scenario.yaml --runs 3       # 빠른 시험
cawit verify scenario.yaml                # 설정한 회차만큼
```

- **수정 전 모두 `FAIL`:** 좋습니다. 경쟁이 재현됐습니다.
- **`NOT_REACHED`:** 훅에 닿지 않았거나 붙잡기가 시간 초과됐습니다. [문제 해결](troubleshooting.md#not_reached)을 보십시오.
- **수정 전인데 `PASS`:** 코드가 이 순서에 이미 안전하거나, 시나리오가 생각한 경로를 지나지 않는 것입니다. `--show-steps`를 확인하고, 애플리케이션 로그로 읽기가 정말 미스였는지 확인하십시오.

## 7. 보관하기

시나리오를 테스트 옆에 커밋하고 CI에서 돌리십시오([CI에서 실행](ci.md)). 사례 연구에서 패턴 20회는 15~30초 걸렸습니다.
