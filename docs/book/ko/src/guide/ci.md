# CI에서 실행

`cawit verify`는 의미 있는 종료 코드와 JUnit 리포트를 내는 평범한 명령이므로 어느 CI에서든 돌릴 수 있습니다.

| 종료 코드 | 의미 | CI는 |
|---|---|---|
| `0` | 모든 회차 통과 | 통과 |
| `1` | 회차 실패: 경쟁이 낡은 값을 남겼거나 캐시가 다시 채워지지 않음 | 실패 |
| `2` | 실패는 없지만 버린 회차가 있음(`NOT_REACHED`, `ERROR`), 또는 시나리오를 실행할 수 없음 | 실패: 테스트가 아무것도 시험하지 않음 |

작업에 필요한 것:

- 훅을 켠(`CAWIT_SYNC_URL`) 애플리케이션. 보통 인스턴스 두 개
- 애플리케이션이 쓰는 PostgreSQL과 Redis. 실행기에서 닿아야 함
- 시나리오 파일과, 시나리오가 읽는 환경 변수(`${env.X}`)

## GitHub Actions

```yaml
name: cache-races

on: [pull_request]

jobs:
  verify:
    runs-on: ubuntu-24.04
    services:
      postgres:
        image: postgres:16
        env: { POSTGRES_PASSWORD: postgres }
        ports: ["5432:5432"]
        options: --health-cmd pg_isready --health-interval 2s --health-retries 30
      redis:
        image: redis:7.2
        ports: ["6379:6379"]
    env:
      CAWIT_SYNC_URL: redis://127.0.0.1:6379
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with: { node-version: 22 }
      - run: npm ci && npm run build
      - run: npm run db:migrate && npm run db:seed
      - name: Start two instances
        run: |
          PORT=3000 npm start > app-0.log 2>&1 &
          PORT=3001 npm start > app-1.log 2>&1 &
          npx wait-on http://127.0.0.1:3000/health http://127.0.0.1:3001/health
      - run: cargo install cawit --locked
      - run: cawit verify tests/cache/stale-set.yaml --junit cawit-report.xml --out cawit-report.json
      - uses: actions/upload-artifact@v4
        if: always()
        with:
          name: cawit-report
          path: |
            cawit-report.xml
            cawit-report.json
            app-*.log
```

## GitLab CI

```yaml
cache-races:
  image: node:22
  services:
    - name: postgres:16
      alias: postgres
    - name: redis:7.2
      alias: redis
  variables:
    POSTGRES_PASSWORD: postgres
    CAWIT_SYNC_URL: redis://redis:6379
  script:
    - npm ci && npm run build && npm run db:migrate && npm run db:seed
    - (PORT=3000 npm start &) && (PORT=3001 npm start &) && npx wait-on http://127.0.0.1:3000/health http://127.0.0.1:3001/health
    - curl -sSf https://sh.rustup.rs | sh -s -- -y && . "$HOME/.cargo/env" && cargo install cawit --locked
    - cawit verify tests/cache/stale-set.yaml --junit cawit-report.xml
  artifacts:
    when: always
    reports:
      junit: cawit-report.xml
```

이 경우 시나리오의 `redis`와 `postgres`는 서비스 호스트(`redis`, `postgres`)를 가리켜야 합니다.

## 리포트

- `--junit`은 회차마다 테스트 케이스 하나를 씁니다. 실패한 회차는 `<failure>`, 버린 회차는 이유와 함께 `<skipped>`입니다.
- `--out`은 전체 JSON 리포트를 씁니다([리포트 형식](../reference/report-format.md)).

## 팁

- **회차 수를 고정하십시오.** 20회면 회귀가 분명히 드러나고, 사례 연구에서는 1분이 훨씬 안 걸렸습니다. 재현에는 1회로도 충분하지만, 회차가 많으면 경쟁이 의도대로 강제되지 않는 설정을 잡아냅니다.
- **`NODE_ENV=production`**이면 CI 작업에 `CAWIT_ALLOW_PRODUCTION=1`이 없는 한 훅이 꺼집니다.
- **콜드 스타트**는 첫 요청을 느리게 만듭니다. 시험 대상 코드가 타이머에 기대면 시나리오에서 시간을 고정하십시오([경쟁 패턴](patterns.md#시간-매개변수)).
- **번들된 빌드**: `cawit-hook`을 업그레이드한 뒤 다시 빌드하거나 외부 의존성으로 지정하십시오([Node 훅](node-hook.md#번들러)).
- **설치 시간**: `cargo install`은 소스에서 빌드합니다(1~2분). 작업 사이에 `~/.cargo`를 캐시하십시오. 미리 빌드한 바이너리는 계획 중입니다.
