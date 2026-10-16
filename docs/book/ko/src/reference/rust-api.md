# Rust API

실행기는 crate 세 개로 이뤄져 있습니다. API 문서는 rustdoc으로 생성하며, 영어로만 제공합니다.

| Crate | 용도 | 문서 |
|---|---|---|
| `cawit` | 명령줄 | [docs.rs/cawit](https://docs.rs/cawit) |
| `cawit-verify` | 엔진: 시나리오, 패턴, 단계, 판정, 실행기 | [docs.rs/cawit-verify](https://docs.rs/cawit-verify) |
| `cawit-resp` | 최소 RESP2/RESP3 파서와 비동기 Redis 클라이언트 | [docs.rs/cawit-resp](https://docs.rs/cawit-resp) |

문서 사이트는 현재 `main` 브랜치의 API 문서도 [`/api/`](https://asyncnomad.github.io/CacheWitness/api/cawit_verify/) 아래에 둡니다.

로컬에서 빌드:

```bash
cargo doc --workspace --no-deps --open
```

실행기 내장하기:

```rust,ignore
use cawit_verify::{Scenario, runner::Options};

let scenario = Scenario::load("stale-set.yaml".as_ref())?;
let report = cawit_verify::run(&scenario, &Options::default(), |run| {
    println!("run {}: {:?}", run.run, run.outcome);
})
.await?;
std::process::exit(report.exit_code());
```

단계 종류, 판정, 패턴을 직접 추가하려면 [엔진 확장](../architecture/extending.md)을 보십시오. API는 0.x 릴리스마다 바뀔 수 있으며, 호환성을 지키는 대상은 시나리오 형식과 훅 프로토콜입니다.
