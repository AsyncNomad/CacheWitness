# 快速开始

本页在一个已有的、在 PostgreSQL 前使用 Redis 缓存的 Node 应用中，加入一个强制触发典型 stale-set 竞态的测试。大约需要十五分钟。

你需要：

- 能在本地以**两个实例**（两个端口）运行你的应用，共享同一个 PostgreSQL 和同一个 Redis（一个实例也可以，见[编写场景](guide/writing-scenarios.md#5-一个实例还是两个)）；
- 一个读取缓存项的接口和一个更新该项的接口；
- Rust（用于安装执行器）和 Node 18 或更高版本。

## 1. 安装

```bash
cargo install cawit                 # 执行器：单个二进制
npm install --save-dev cawit-hook   # 钩子：无依赖
```

> 在 v0.1 发布之前，请从仓库构建执行器（`cargo build --release`，二进制位于 `target/release/cawit`），并从 `hooks/node` 安装钩子。

## 2. 找到回填路径并加入钩子

找到缓存未命中时执行的代码：它读取数据库，然后写入缓存。在**读取和写入之间**加一行调用：

```ts
import * as cawit from "cawit-hook";

async function getArticle(id: string) {
  const key = `article:${id}`;
  const cached = await redis.get(key);
  if (cached) return JSON.parse(cached);

  const row = await db.query("select * from articles where id = $1", [id]);
  await cawit.syncPoint("refill:after-read", `/articles/${id}`);   // ← 钩子
  await redis.set(key, JSON.stringify(row), "EX", 300);
  return row;
}
```

第一个参数是同步点的名字；第二个是键，场景可以用它只暂停某一个特定的回填。任何字符串都可以，这里用的是请求路径，方便在场景中书写。

未设置 `CAWIT_SYNC_URL` 时，这个调用什么也不做（不做任何 I/O）。缓存库、打包工具和生产环境保护见 [Node 钩子](guide/node-hook.md)。

## 3. 启用钩子并启动应用

```bash
export CAWIT_SYNC_URL=redis://127.0.0.1:6379   # 与执行器使用的是同一个 Redis
PORT=3000 npm start &
PORT=3001 npm start &
```

## 4. 描述竞态

创建 `stale-set.yaml`：

```yaml
name: article-stale-set
runs: 20
apps:
  a: http://127.0.0.1:3000
  b: http://127.0.0.1:3001
redis: redis://127.0.0.1:6379
postgres: "host=127.0.0.1 user=postgres password=postgres dbname=app"

requests:
  update:
    method: PATCH
    path: /articles/1
    json: { title: "${value}" }
  read:
    method: GET
    path: /articles/1

state:
  db_query: "select title from articles where id = 1"
  cache_key: "article:1"
  cache_field: title

pattern:
  stale-set:
    reader: a
    writer: b
    write: update
    read: read
    key: /articles/1
```

- `requests` 是场景可以发送的 HTTP 调用。写请求必须在新值的位置使用 `${value}`；每一轮写入的值都是唯一的。
- `state` 说明要比较什么：存放在 `article:1` 下的 JSON 的 `title` 字段，与数据库中的 `title`。如果缓存键是哈希或值经过压缩，请改用 `app_view` 通过应用读取（[场景格式](reference/scenario-format.md#状态)）。
- `pattern: stale-set` 会展开为强制触发竞态的步骤（[竞态模式](guide/patterns.md#stale-set)）。

查看究竟会执行什么：

```bash
cawit verify stale-set.yaml --show-steps
```

## 5. 运行

```bash
cawit verify stale-set.yaml
```

没有修复时，每一轮都应失败：

```text
run   1  FAIL
          FAIL not_stale (cache_not_stale): cache=…-1-a db=…-1-b
          FAIL refilled (cache_equals_db): cache=…-1-a db=…-1-b
...
article-stale-set: 20 runs  pass 0  fail 20  not_reached 0  error 0
  not_stale            passed 0/20
  refilled             passed 0/20
```

退出码为 `1`。这就是按需复现的竞态。

如果轮次显示的是 `NOT_REACHED`，说明应被暂停的请求没有到达钩子，见[故障排查](guide/troubleshooting.md#not_reached)。

## 6. 修复并证明修复

应用一个修复（例如租约或版本代号检查，见[竞态模式](guide/patterns.md#能通过的修复与不能通过的修复)），重启应用，运行同一个场景。正确的修复会通过每一轮并以 `0` 退出。把场景放进仓库，在 CI 中运行（[在 CI 中运行](guide/ci.md)）。

## 下一步

- [核心概念](guide/concepts.md)：同步点、轮次、操纵检查、判定。
- [竞态模式](guide/patterns.md)：三种内置竞态，以及哪些修复能经受住它们。
- [案例研究](case-studies/directus.md)：针对 Twenty 和 Directus 的完整场景。
