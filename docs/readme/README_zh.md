<div align="center">

[English](../../README.md) · **简体中文** · [한국어](README_ko.md)

# CacheWitness

**按需复现缓存并发缺陷，并证明修复确实有效**

**缓存见证者**

</div>

---

## 简介 (Introduction)

   **CacheWitness**（CLI：`cawit`）是一款面向 PostgreSQL + Redis 应用的 Rust 开源测试工具。它**让缓存竞态每次都以完全相同的顺序发生，并在 CI 中证明修复确实能阻止它。** 一句话概括：**“每次都以同样的方式触发缓存竞态，并证明修复真的能阻止它。”**

   在数据库前放一层 Redis 的 cache-aside 架构是 Web 后端的默认选择，其中的竞态也广为人知：读者读到旧行，写者提交并使缓存失效，读者又把旧行写回去。问题在于这类竞态是**时序上的偶然**。它们在测试中很少出现，一加日志就消失，而且**无法针对一个可能发生也可能不发生的偶然去测试修复。** 我们在真实应用上实测过：同样的负载下，一次运行在 6.5% 的写入突发后留下了陈旧值，另一次只有 0.17%。

   **CacheWitness 的做法是控制顺序，而不是等待它出现。** 缓存回填路径中的一个小同步钩子会让读者在读完数据库后立即暂停。场景执行器随后通过另一个请求完成提交和失效，再放行读者，并把 Redis 与数据库进行比对。竞态每次都会发生，因此测试在修复前每次都失败，修复后每次都通过。CacheWitness 基于一个明确的理念设计：**“不等竞态出现。主动触发它，并证明修复。”**

   它面向**缓存库和 ORM 的作者，以及自己编写缓存代码的团队**，适用于需要不会时好时坏的缓存失效回归测试的场景。

---

## 要解决的问题 (The Problem)

| 竞态 | 发生了什么 | 常见但不够的“修复” |
|---|---|---|
| stale-set 竞态 | 读者读到旧值 → 写者提交并 DEL → 读者 SET 旧值 | SET 前比较版本：DEL 之后已没有可比较的值 |
| 提交前失效 | 事务内执行 DEL → 提交前另一个请求用旧值回填 | 延迟双删：只是缩小窗口，无法关闭 |
| 失效失败 | DEL 因网络、权限失败且未重试 | 短 TTL：只能限制损害，不能修复 |
| 副本延迟回填 | DEL 之后，延迟的只读副本返回旧值 | 未命中时从主库读：前提是所有回填路径都这样做 |

安全的模式（租约 lease、带版本号的失效标记）是已知的，**它们的陷阱也是已知的。** 在我们第一次真实应用实验中，两个看似正确的修复其实都不对：版本比较 SET 毫无作用；没有重新读取的租约会让进程内的 memoizer 在 10 秒内继续返回旧值。只有强制触发竞态的测试才能显示出这种差别。

---

## 快速开始 (Quick Start)

> v0.1 尚未发布。在此之前，请用 `cargo build --release` 构建执行器，并使用 [`hooks/node`](../../hooks/node) 中的钩子。针对真实应用的可运行示例：[`lab/twenty/`](../../lab/twenty/)（Twenty CRM）和 [`lab/directus/`](../../lab/directus/)（Directus）。

### 1. 安装

```bash
cargo install cawit                 # 执行器，单个二进制
npm install --save-dev cawit-hook   # 钩子，无依赖
```

### 2. 在回填路径中加入同步钩子

在读完数据库之后、写入缓存之前加一行调用。只有设置了 `CAWIT_SYNC_URL` 且测试启用它时才会生效（`NODE_ENV=production` 下除非设置 `CAWIT_ALLOW_PRODUCTION=1`，否则保持关闭）。

```ts
import * as cawit from "cawit-hook";

const row = await db.findOne(id);
await cawit.syncPoint("refill:after-read", key);
await cache.set(key, serialize(row));
```

### 3. 描述竞态并运行

```yaml
# stale-set.yaml
name: stale-set
runs: 20
apps: { a: "http://127.0.0.1:3000", b: "http://127.0.0.1:3001" }
redis: redis://127.0.0.1:6379
postgres: "host=127.0.0.1 user=postgres password=postgres dbname=app"
requests:
  update: { method: PATCH, path: /articles/1, json: { title: "${value}" } }
  read:   { method: GET,   path: /articles/1 }
state:
  db_query: "select title from articles where id = 1"
  app_view: { app: a, request: read, field: /title, hit_header: x-cache }
pattern:
  stale-set: { reader: a, writer: b, write: update, read: read, key: /articles/1 }
```

```bash
# 以 CAWIT_SYNC_URL=redis://127.0.0.1:6379 启动应用，然后：
cawit verify stale-set.yaml --junit report.xml   # 只要有一轮留下陈旧值，退出码为 1
cawit verify stale-set.yaml --show-steps          # 查看模式展开后的步骤
```

每一轮按以下顺序进行：

1. 通过服务器 A 写入（提交并失效）；
2. 启用钩子并通过 A 读取：A 未命中，读取数据库后停在钩子处；
3. 通过服务器 B 写入更新的值（提交并失效）；
4. 放行 A，A 写入它读到的值；
5. 检查读者拿到的是数据库已提交的值；
6. 检查缓存会以当前值重新回填（修复不能让缓存彻底失效）。

没有到达钩子、或暂停已超时的轮次会被丢弃，不计为通过。模式、步骤与判定：[`docs/scenarios.md`](../scenarios.md)（英文）。

### 4. 查看结果

```
stale-set（20 轮，每轮都到达了钩子）
  修复前                         陈旧 20/20                 FAIL
  版本比较 SET                   陈旧 20/20                 FAIL  DEL 之后没有可比较的值
  租约                           陈旧  0/20，回填  0/20     FAIL  从内存返回旧值
  租约 + 被拒后重新读取          陈旧  0/20，回填 20/20     PASS
```

---

## 设计理念 (Design Philosophy)

### 1. 控制顺序，而不是抽样

概率性的压力测试问的是“多常失败？”，答案每次运行都不同。强制顺序的测试问的是“在这个顺序下会失败吗？”，答案每次都一样。这正是它能作为 CI 回归测试的原因。

### 2. 每一轮都证明自己测到了东西

如果钩子从未被触达，竞态就没有发生，通过也就毫无意义。每一轮都会记录是否到达钩子，未到达的轮次会被丢弃（操纵检查）。

### 3. 用状态判定，用顺序解释

判定很简单：Redis 中的值是否是数据库已提交的值，之后缓存能否重新回填。失败时，CacheWitness 将（计划中）用观测到的事件顺序（Redis 执行顺序与 PostgreSQL 提交顺序）来解释经过，而不是墙上时钟的时间戳——我们的测量表明后者并不可靠。

### 4. 为什么选择 Rust

- **与应用语言无关的单个二进制：** 执行器无需 Node 或 Python 运行时即可放入任何 CI 运行器，钩子则是各语言的小型库。
- **坦白说，执行器本身用任何语言都能写：** 它主要是发送请求并在 Redis 上等待。Rust 的价值在于计划中的观测引擎（Redis 协议代理、PostgreSQL 逻辑复制读取器），它需要可预测的延迟和内存上限，并且要解析不可信的流量。

---

## 主要功能 (Key Features)

- **🎯 确定性复现：** 已知的缓存竞态模式，每次都以相同顺序触发
- **✅ 修复验证：** 修复前失败、修复后通过，并检查缓存仍能回填
- **🧪 内置模式：** stale-set、失效失败（故障注入）、提交前失效；副本延迟回填为计划中
- **🔍 操纵检查：** 未到达钩子的轮次会被丢弃
- **🧾 基于顺序的解释（计划中）：** 失败的轮次附带提交、删除、写入的观测顺序。目前失败轮次会给出读者拿到的值与数据库中的值
- **👀 辅助功能，check 模式（无需修改应用代码）：** 通过观察 Redis 与 PostgreSQL WAL 确认陈旧的缓存状态

---

## 实测结果 (What We Measured)

所有实验均在本地运行，应用实验环境与网络隔离。详情见 [`docs/design/experiments-summary.md`](../design/experiments-summary.md)（韩文）。

- **在 Twenty CRM 上强制顺序（各 20 轮）：** 修复前 20/20 陈旧；版本比较 SET 20/20 陈旧；租约 0/20 陈旧但没有回填（进程内 memoizer）；租约 + 重新读取 0/20 陈旧、20/20 回填。
- **在 Directus 上强制顺序（各 20 轮）：** 修复前 20/20 陈旧；延迟双删在读者慢于延迟时 20/20 陈旧（读者快时 10/10 通过，因此能逃过普通测试）；版本代号检查 20/20 通过。
- **用概率代替顺序：** 同样的突发写入负载，一次运行在 6.5% 的突发后留下陈旧值，另一次为 0.17%。通过代理延迟缓存写入，比真实的慢回填高估最多 10~30 倍。
- **在真实测试套件上做检测：** 1,528 次缓存行变更中有 1,527 次被失效（中位数 1.7ms）。基准实验中陈旧状态全部被确认（157/157），但确定性的失效缺陷很少见。

正是这些结果让 CacheWitness 专注于强制复现和修复验证。

---

## 与现有工具的关系 (Related Tools)

在指定位置暂停代码以强制顺序，这种技术本身并不新。CacheWitness 的不同之处在于暂停点所在的位置以及它判定的对象。

| 工具 | 作用 | 区别 |
|---|---|---|
| RocksDB SyncPoint、fail-rs（TiKV）、failpoint（Go） | 编译进单个进程的同步点与故障点 | CacheWitness 通过 Redis 跨进程、跨服务器协调，并由 HTTP 请求驱动场景 |
| Lincheck、Fray（JVM）、Coyote（.NET） | 在单个运行时内探索线程交错 | CacheWitness 在连接真实数据库与 Redis 的请求层面工作 |
| Jepsen | 在故障下对分布式数据库做黑盒一致性测试 | CacheWitness 针对应用的缓存逻辑，提供缓存专用的模式与判定 |

---

## 范围 (Scope)

| 项目 | 首个公开版本 (v0.x) | 之后 |
|---|---|---|
| 钩子 | Node/TS | Python，其他语言 |
| 场景 | stale-set、提交前失效、失效失败 | 副本延迟回填、列表/聚合类缓存 |
| 数据库 / 缓存 | PostgreSQL、Redis/Valkey 单节点 | MySQL、Redis Cluster |
| check 模式（辅助） | 在 verify 引擎之后 | — |

**CacheWitness 不做的事：**

- 不会发现没有告诉它的竞态：verify 是针对已知顺序的回归测试。
- 不估计竞态在生产环境中发生的频率。
- 不修复也不替换你的缓存。

---

## 文档 (Documentation)

| 文档 | 内容 |
|---|---|
| [`docs/PLAN.md`](../PLAN.md) | 开发计划（韩文）：产品定义、设计、分阶段关卡、被否决的方案 |
| [`docs/design/experiments-summary.md`](../design/experiments-summary.md) | 迄今为止的所有实验及其带来的改变（韩文） |

---

## ⚠️ 免责声明与责任限制 (Disclaimer & Liability)

**请务必仔细阅读。**

**CacheWitness 目前是处于设计和原型阶段的个人项目。**

本软件**按“原样”（as is）提供，不附带任何形式的担保**，包括但不限于适销性、特定用途适用性和不侵权的担保。使用本软件即表示你同意以下事项：
1. **自担风险：** 你需自行判断使用或再分发本软件是否合适，并承担全部责任。
2. **免除责任：** 对于因使用本软件或与之相关而产生的任何索赔、损害或其他责任，作者或版权持有人概不负责。
3. **仅限测试环境：** 同步钩子和场景执行器仅用于测试环境。切勿在生产环境中启用钩子。

---

## 许可证 (License)

本项目基于 **Apache License 2.0** 发布，但 `lab/` 下修改其他项目（Twenty、Directus）的实验补丁除外，它们遵循各自项目的许可证，详见 [`THIRD_PARTY_NOTICES.md`](../../THIRD_PARTY_NOTICES.md)。Twenty 与 Directus 是其各自所有者的商标；CacheWitness 与它们没有关联，也未获其认可。
