# 案例研究：Directus

[Directus](https://github.com/directus/directus) 是一个提供 REST 和 GraphQL API 的数据平台（Express、knex）。在 `CACHE_ENABLED=true`、`CACHE_STORE=redis`、`CACHE_AUTO_PURGE=true` 时，它把完整的 GET 响应缓存在 Redis 中，并在每次写入后清除整个缓存命名空间。它的缓存在每一层都与 Twenty 不同，因此是很好的第二个目标。

| | Twenty | Directus |
|---|---|---|
| 框架 / ORM | NestJS / TypeORM | Express / knex |
| 缓存单位 | 实体 | HTTP 响应体 |
| 键 | `…:workspace:<id>:data` | 版本、用户、路径、查询的哈希 |
| 值 | JSON | 压缩后 |
| 失效 | 删除一个键 | 清除整个命名空间 |
| 进程内缓存 | 本地缓存 + memoizer | 无 |

版本：v12.5.0（`4f77933b`），两个共享 Redis 的 API 实例。所有运行均在本地且与网络隔离。实验文件：[`lab/directus/`](https://github.com/AsyncNomad/CacheWitness/tree/main/lab/directus)。

## 钩子

| 同步点 | 位置 | 行数 |
|---|---|---|
| `refill:after-read` | `middleware/respond.ts`，在 `setCacheValue` 之前，键为 `req.baseUrl + req.path` | 3 |
| `invalidate:before` | `cache.ts` 中的数据缓存清除 | 2 |
| `write:before-commit` | `ItemsService.updateMany` 中事务的末尾 | 2 |

因为键是哈希、值经过压缩，所以场景通过应用来判定：`state.app_view` 从 `GET /items/articles/1` 读取 `/data/title`，并用 `X-Cache-Status`（`CACHE_STATUS_HEADER`）区分命中与数据库读取。

## 结果（各 20 轮，每一轮都到达了同步点）

### stale-set

| 修复 | 结果 |
|---|---|
| 无 | FAIL 20/20：旧标题以缓存命中的形式返回 |
| 延迟的第二次清除（500ms），读者暂停 1 秒 | FAIL 20/20 |
| 代号检查（清除前递增计数器；丢弃在旧代号下开始的回填） | PASS 20/20 |

对照实验：立即放行读者时，延迟的第二次清除**10/10 通过**。它只在读者慢于延迟时失败，而普通测试恰恰不会制造这种情况。用一个实例代替两个时：无修复 FAIL 10/10，代号检查 PASS 10/10。

### failed-invalidation

| 修复 | 严格 | 3 秒内 |
|---|---|---|
| 无（TTL 5 分钟） | FAIL 20/20 | FAIL 20/20 |
| 短 TTL（2 秒） | FAIL 20/20 | PASS 20/20 |
| 重试清除 | PASS 20/20 | PASS 20/20 |

没有修复时，写入在提交之后返回 500：客户端看到的是失败，数据库里是新值，而缓存保留旧值。

### invalidate-before-commit

| 配置（`hold_ms: 1000`） | 结果 |
|---|---|
| 原版 Directus（提交后清除） | PASS 20/20 |
| 把清除移入事务（实验用缺陷） | FAIL 20/20 |
| 在此基础上再加延迟的第二次清除 | FAIL 20/20 |

原版通过：模式不会误报正确的代码。

## 不强制顺序时有多常见？

两个实例，两边都有读者，一个写者，没有钩子也没有修复；每次写入（或一组连续写入）之后 300ms 进行一次检查读取。每种条件独立运行两次：

| 写入方式 | 每实例读者数 | 第 1 次运行 | 第 2 次运行 |
|---|---|---|---|
| 间隔 0.5 秒 | 2 或 8 | 0/300、0/300 | 0/300、0/300 |
| 连续 2 次 | 2 | 5.0% | 2.0% |
| 连续 2 次 | 8 | 0.5% | 0% |
| 连续 5 次 | 2 | 6.0% | 2.5% |
| 连续 5 次 | 8 | 1.0% | 0.5% |

这个竞态需要在写入到达时有一个正在进行的回填，所以只有在快速连续保存时才会出现；此时上一个值可能会以缓存命中的形式一直保留，直到下一次写入或 `CACHE_TTL` 到期。

## 我们学到了什么

- **打包工具可能把旧钩子固化下来。** Directus 的构建把钩子复制进了产物。协议变更后每一轮都是 `NOT_REACHED`；Redis `MONITOR` 显示的是旧脚本。解决办法是把 `cawit-hook` 标记为外部依赖。
- **路由相对路径。** 最初几轮是 `NOT_REACHED`，因为挂载路由内部的 `req.path` 不含挂载前缀。操纵检查把一个无声的配置错误变成了可见的问题。
- **共享的等待连接破坏了单实例场景。** 早期的钩子在共享连接上等待，同一进程中的第二个请求排在被暂停的请求之后，直到暂停超时，该轮在顺序已被破坏的情况下通过了。现在每次暂停都在自己的连接上等待，并且执行器会丢弃暂停已超时的轮次。
- **定时器需要固定的时间。** 没有 `hold_ms` 时，延迟清除的修复在 20 轮中翻转了一次，发生在冷启动的第一轮。见[竞态模式](../guide/patterns.md#时间参数)。
