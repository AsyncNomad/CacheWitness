# Node 钩子

`cawit-hook` 是[钩子协议](../reference/hook-protocol.md)的 Node 实现。它没有依赖，并附带 TypeScript 类型。

```bash
npm install --save-dev cawit-hook
```

```ts
import * as cawit from "cawit-hook";          // ESM / TypeScript
// const cawit = require("cawit-hook");       // CommonJS
```

## API

### `syncPoint(point: string, key?: string): Promise<void>`

一个具名位置，场景可以在这里暂停调用或让它失败。

- `point`：场景使用的同步点名称（`refill:after-read`、`invalidate:before`、`write:before-commit`，或你自己定义的名称）。
- `key`：可选。场景可以只为某一个键启用该点；其他键的调用会直接通过，不影响启用状态。默认为 `*`。

行为：

| 状态 | 调用做什么 |
|---|---|
| 钩子关闭（没有 `CAWIT_SYNC_URL`，或生产环境保护） | 立即返回，不做 I/O |
| 已开启，该点未启用（或为其他键启用） | 一次 Redis 往返，然后返回 |
| 以 `hold` 启用 | 通知执行器，然后等待直到被放行（或到 `CAWIT_SYNC_TIMEOUT_S`） |
| 以 `fail` 启用 | 通知执行器，然后抛出 `CawitInjectedError`（一次） |

### `enabled: boolean`

钩子在本进程中是否处于开启状态（加载时读取一次）。

### `CawitInjectedError`

以 `fail` 启用的同步点被取走时抛出的错误。

### `close(): void`

关闭钩子共享的 Redis 连接（用于测试和优雅关闭）。

## 环境变量

| 变量 | 默认值 | 含义 |
|---|---|---|
| `CAWIT_SYNC_URL` | 未设置 | 执行器也使用的 `redis://[[user]:password@]host[:port][/db]`，TLS 用 `rediss://`。未设置表示钩子关闭 |
| `CAWIT_SYNC_TIMEOUT_S` | `30` | 被暂停的调用等待放行的时长（可用小数）。执行器把超时的暂停视为 `NOT_REACHED` |
| `CAWIT_ALLOW_PRODUCTION` | 未设置 | 在 `NODE_ENV=production` 下，除非此值为 `1`，否则钩子保持关闭 |
| `CAWIT_SYNC_TLS_INSECURE` | 未设置 | `1`：不验证 `rediss://` 服务器的证书（仅限测试服务器） |

使用 `rediss://` 时，钩子信任 Node 的 CA 列表以及 `NODE_EXTRA_CA_CERTS`。URL 中带密码时，钩子在它打开的每个连接上发送 `AUTH`；`AUTH` 失败会让调用带着原因抛出异常（绝不打印密码）。

## 调用放在哪里

**回填（stale-set）。** 在读取数据库和写入缓存之间：

```ts
const row = await repo.findOneBy({ id });
await cawit.syncPoint("refill:after-read", `article:${id}`);
await cache.set(`article:${id}`, row, ttl);
```

如果使用 `cache.wrap(key, loader)` 这类读穿透辅助函数，就把调用放在 loader 的末尾、读取数据库之后：

```ts
return cache.wrap(`article:${id}`, async () => {
  const row = await repo.findOneBy({ id });
  await cawit.syncPoint("refill:after-read", `article:${id}`);
  return row;
});
```

**失效（failed-invalidation）。** 紧挨在删除缓存项的调用之前：

```ts
await cawit.syncPoint("invalidate:before", `article:${id}`);
await cache.del(`article:${id}`);
```

**提交前（invalidate-before-commit）。** 作为事务内的最后一条语句：

```ts
await db.transaction(async (trx) => {
  await trx("articles").update({ title }).where({ id });
  await cache.del(`article:${id}`);              // 被测试的缺陷
  await cawit.syncPoint("write:before-commit", `article:${id}`);
});
```

## 连接

钩子绝不使用应用自己的 Redis 连接。非阻塞命令共享一个钩子连接；每个被暂停的调用都在自己的连接上等待，因此被暂停的请求不会拖慢同一进程中的其他请求或其他同步点。钩子连接在开启状态下首次调用时才建立。

## 打包工具

如果你的构建会打包依赖（tsdown、esbuild、webpack、ncc），请把 `cawit-hook` 标记为外部依赖，或在升级后重新构建。构建产物中残留的旧版钩子使用旧协议，永远取不走启用，每一轮都会报告 `NOT_REACHED`。这在 [Directus 案例研究](../case-studies/directus.md#我们学到了什么)中实际发生过。

## 生产环境安全

- 没有 `CAWIT_SYNC_URL` 时，调用在做任何事之前就返回。
- 在 `NODE_ENV=production` 下，即使设置了 `CAWIT_SYNC_URL`，除非 `CAWIT_ALLOW_PRODUCTION=1`，钩子也保持关闭。只在以 `NODE_ENV=production` 运行的测试环境（类似预发布的端到端测试）中设置它。
- 即使已开启，未启用的同步点每次调用也有一次 Redis 往返的开销。不要在在意这点延迟的地方开启钩子。

## 其他语言

协议只是几个 Redis 命令（[钩子协议](../reference/hook-protocol.md)）。借助现有的 Redis 客户端库，其他语言的钩子只需几十行（Node 钩子约 150 行，因为它为了零依赖自带了 RESP 解析器）。Python 版本已在计划中。
