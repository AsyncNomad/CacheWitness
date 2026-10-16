# Rust API

执行器由三个 crate 组成。API 文档由 rustdoc 生成，仅提供英文版。

| Crate | 用途 | 文档 |
|---|---|---|
| `cawit` | 命令行 | [docs.rs/cawit](https://docs.rs/cawit) |
| `cawit-verify` | 引擎：场景、模式、步骤、检查、执行器 | [docs.rs/cawit-verify](https://docs.rs/cawit-verify) |
| `cawit-resp` | 最小的 RESP2/RESP3 解析器和异步 Redis 客户端 | [docs.rs/cawit-resp](https://docs.rs/cawit-resp) |

文档站点还在 [`/api/`](https://asyncnomad.github.io/CacheWitness/api/cawit_verify/) 下提供当前 `main` 分支的 API 文档。

在本地构建：

```bash
cargo doc --workspace --no-deps --open
```

嵌入执行器：

```rust,ignore
use cawit_verify::{Scenario, runner::Options};

let scenario = Scenario::load("stale-set.yaml".as_ref())?;
let report = cawit_verify::run(&scenario, &Options::default(), |run| {
    println!("run {}: {:?}", run.run, run.outcome);
})
.await?;
std::process::exit(report.exit_code());
```

要添加自己的步骤类型、检查或模式，见[扩展引擎](../architecture/extending.md)。API 在任何 0.x 版本中都可能变化；兼容性面向的是场景格式和钩子协议。
