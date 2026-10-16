# 환경 변수

## 애플리케이션 (훅)

| 변수 | 기본값 | 의미 |
|---|---|---|
| `CAWIT_SYNC_URL` | 없음 | 훅 프로토콜용 Redis URL, `redis://host[:port][/db]`. 없으면 훅이 꺼지고 I/O를 하지 않음 |
| `CAWIT_SYNC_TIMEOUT_S` | `30` | 붙잡힌 호출이 해제를 기다리는 초. 만료되면 회차는 `NOT_REACHED` |
| `CAWIT_ALLOW_PRODUCTION` | 없음 | `NODE_ENV=production`이면 이 값이 `1`이 아닌 한 훅이 꺼진 채로 있음 |

## 실행기

`cawit` 자체는 환경 변수를 읽지 않습니다. 시나리오는 요청 템플릿에서 `${env.NAME}`으로 어떤 변수든 읽을 수 있고, 설정되지 않은 변수는 오류입니다.
