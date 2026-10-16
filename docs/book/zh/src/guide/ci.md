# 在 CI 中运行

`cawit verify` 是一个普通命令，有含义明确的退出码和 JUnit 报告，因此任何 CI 系统都能运行它。

| 退出码 | 含义 | CI 应当 |
|---|---|---|
| `0` | 所有轮次通过 | 通过 |
| `1` | 有轮次失败：竞态留下了陈旧值，或缓存没有重新回填 | 失败 |
| `2` | 没有失败，但有轮次被丢弃（`NOT_REACHED`、`ERROR`），或场景无法运行 | 失败：测试什么也没测到 |

作业需要：

- 启用了钩子（`CAWIT_SYNC_URL`）的应用，通常为两个实例；
- 应用使用的 PostgreSQL 和 Redis，执行器能够访问；
- 场景文件以及它读取的环境变量（`${env.X}`）。

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

这种情况下，场景中的 `redis` 和 `postgres` 必须指向服务主机名（`redis`、`postgres`）。

## 报告

- `--junit` 为每一轮写一个测试用例。失败的轮次是 `<failure>`，被丢弃的轮次是带原因的 `<skipped>`。
- `--out` 写出完整的 JSON 报告（[报告格式](../reference/report-format.md)）。

## 提示

- **固定轮数。** 20 轮足以让回归一目了然，在案例研究中远不到一分钟。复现只需一轮，但更多轮次能发现竞态没有按预期被强制的配置。
- **`NODE_ENV=production`** 下，除非在 CI 作业中设置 `CAWIT_ALLOW_PRODUCTION=1`，否则钩子是关闭的。
- **冷启动**会让最初的请求变慢。如果被测代码依赖定时器，请在场景中固定时间（[竞态模式](patterns.md#时间参数)）。
- **打包构建**：升级 `cawit-hook` 后重新构建，或将其标记为外部依赖（[Node 钩子](node-hook.md#打包工具)）。
- **安装时间**：`cargo install` 从源码构建（一两分钟）。在作业之间缓存 `~/.cargo`；预编译二进制已在计划中。
