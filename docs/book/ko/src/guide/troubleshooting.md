# 문제 해결

## NOT_REACHED

```text
run   1  NOT_REACHED
          sync point `refill:after-read` not reached within 10000ms
```

장전한 호출이 도착하지 않아 강제하려던 순서가 일어나지 않았습니다. 순서대로 확인하십시오.

1. **애플리케이션에서 훅이 켜져 있는가.** `CAWIT_SYNC_URL`은 실행기가 아니라 애플리케이션의 환경에 있어야 하고, `NODE_ENV=production`이면 `CAWIT_ALLOW_PRODUCTION=1`도 필요합니다. 확실히 하려면 시작할 때 `require("cawit-hook").enabled`를 로그로 남기십시오.
2. **같은 Redis인가.** 애플리케이션의 `CAWIT_SYNC_URL`과 시나리오의 `redis`가 같은 Redis 서버, 같은 DB 번호를 가리켜야 합니다.
3. **요청이 훅을 건 경로를 지나는가.** 캐시에 적중한 읽기는 재충전 훅에 닿지 않습니다. 패턴의 첫 쓰기가 항목을 무효화하지만, Redis 앞의 다른 캐시(프로세스 안 LRU, HTTP 캐시)가 응답할 수도 있습니다. 애플리케이션 로그에서 DB 읽기가 있었는지 확인하십시오.
4. **키가 맞는가.** 시나리오가 `key: X`로 장전했다면 훅은 정확히 `X`로 호출돼야 합니다. 프레임워크가 보는 경로는 클라이언트가 보낸 경로와 다를 때가 많습니다. Express에서 마운트된 라우터 안의 `req.path`에는 마운트 접두사가 빠져 있습니다(`req.baseUrl + req.path`를 쓰십시오).
5. **빌드에 현재 훅이 들어 있는가.** 번들러가 옛 `cawit-hook` 사본을 빌드에 넣었을 수 있습니다. 회차가 도는 동안 Redis를 지켜보십시오.

   ```bash
   redis-cli monitor | grep -E 'cawit:|EVAL'
   ```

   실행기의 `SET cawit:arm:<point>`, 훅의 `EVAL`, `RPUSH cawit:reached:<token>`이 차례로 보여야 합니다. 훅의 `EVAL` 스크립트가 현재 것과 다르면 다시 빌드하십시오.

## "the hold at … timed out before release"

시나리오가 풀어 주기 전에 붙잡힌 호출이 기다리기를 그만둬서 순서가 깨졌고, 회차를 버렸습니다. 시나리오가 `reached`와 `release` 사이에 훅의 `CAWIT_SYNC_TIMEOUT_S`(기본 30초)보다 오래 기다리거나, 그 사이의 단계(HTTP 요청)가 그만큼 걸린 것입니다. 애플리케이션 환경에서 시간 초과를 늘리거나 느린 단계를 찾으십시오.

## ERROR

```text
run   1  ERROR
          step 6 (http): HTTP 500 from http://127.0.0.1:3001/items/articles/1: …
```

요청이 오류 상태(또는 `error_field`를 준 경우 GraphQL `errors` 필드)를 돌려줬거나, 판정이 값을 읽지 못했거나, 단계 설정이 잘못됐습니다. 메모가 단계를 알려 줍니다. 요청이 실패할 것이 예상되면(장애 주입) 그 `http` 단계에 `allow_error: true`를 주십시오. `failed-invalidation` 패턴은 이것을 대신 해 줍니다.

## 수정 전인데 모든 회차가 통과

- 이 순서에 대해서는 경쟁이 정말 닫혀 있을 수 있습니다(예: 커밋 뒤에 무효화하는 코드에 `invalidate-before-commit` 패턴).
- 또는 시나리오가 그 경로를 지나지 않습니다. `--show-steps`로 확인하고, 읽기가 정말 미스였다가 다시 채웠는지(애플리케이션 로그), `state`가 맞는 값을 비교하는지 보십시오. `app_view`에 `hit_header`를 주면 `refilled` 판정이 그 값이 캐시에서 왔음을 증명합니다.

## 회차마다 결과가 다름

시나리오가 통제하는 순서와 대기 안에서는 회차가 결정적입니다. 결과가 갈린다면:

- 시험 대상 코드가 타이머에 기대고 있을 가능성이 큽니다. `hold_ms`로 시간을 고정하거나 `within_ms`를 쓰십시오([경쟁 패턴](patterns.md#시간-매개변수)).
- 기동 직후 첫 회차가 느립니다(콜드 캐시, JIT, 연결 풀).
- 시나리오가 같은 시간에 도는 다른 무언가와 DB나 캐시를 공유합니다.

## "unknown variable ${…}" 또는 "environment variable … is not set"

템플릿은 `${...}`를 글자 그대로 보내는 대신 크게 실패합니다. 단계의 `vars`에 변수를 정의하거나, `${run}`/`${run_index}`를 쓰거나, `cawit` 프로세스에 환경 변수를 내보내십시오.

## "unknown field …"

시나리오 파일은 모든 단계에서 모르는 키를 거부하므로, 오타는 로드할 때 실패합니다. [시나리오 형식](../reference/scenario-format.md)과 비교하십시오.
