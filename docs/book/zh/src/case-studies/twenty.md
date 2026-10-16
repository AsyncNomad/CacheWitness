# 案例研究：Twenty CRM

[Twenty](https://github.com/twentyhq/twenty) 是一款开源 CRM（NestJS、TypeORM、PostgreSQL、Redis）。我们测试了它的核心实体缓存，它缓存每个已认证请求都会读取的工作区记录。

所有运行都在本地进行，应用实验环境与网络隔离。固定提交：`c3ba1401`。实验文件：[`lab/twenty/`](https://github.com/AsyncNomad/CacheWitness/tree/main/lab/twenty)。

## 缓存结构

- 未命中时，`CoreEntityCacheService.recomputeFromProvider` 从 PostgreSQL 读取实体，并无条件写入 Redis（`MSET`）。
- 失效在写入之后删除缓存项。
- Redis 前面还有两层进程内缓存：每 100ms 重新校验一次的本地缓存，以及把已解析的值保留 10 秒的 memoizer。

## 钩子

在缓存服务中加了四行：一行注释、一行 import，以及

```ts
await cawit.syncPoint('refill:after-read', `${cacheKeyName}:${entityId}`);
```

位置紧接在提供者读取数据库之后。场景以 `workspaceEntity:<工作区 id>` 启用，因为一个请求会回填多个实体（用户、工作区、API 密钥），而只有工作区的回填应被暂停。

## 场景

[`lab/twenty/scenarios/stale-set.yaml`](https://github.com/AsyncNomad/CacheWitness/blob/main/lab/twenty/scenarios/stale-set.yaml)：两台服务器、一个重命名工作区的 GraphQL 变更、一个读取它的查询，以及把缓存中的 `displayName` 与数据库比较的 `state`。GraphQL 的错误以状态 200 返回，所以两个请求都设置了 `error_field: errors`。

## 结果（各 20 轮，每一轮都到达了钩子）

| 候选修复 | not_stale | refilled | 结果 |
|---|---|---|---|
| 无 | 0/20 | 0/20 | FAIL |
| 版本比较 `SET`（仅当缓存中的副本不更新时才写入） | 0/20 | 0/20 | FAIL |
| 租约（失效会删除租约；仅当租约仍在时才写入） | 20/20 | 0/20 | FAIL |
| 租约 + 丢失租约时重新读取 | 20/20 | 20/20 | PASS |

## 我们学到了什么

- **版本比较 `SET` 在这里毫无作用。** 更新写入的 `DEL` 之后缓存为空，没有更新的副本可供比较，于是旧值照样被写入。
- **只用租约看似正确，却会破坏缓存。** 它阻止了陈旧写入，但丢失租约的请求会返回它读到的值，memoizer 在 Redis 为空的情况下继续返回该值 10 秒。只有 `refilled` 判定能发现这一点。
- **自然发生频率很低。** 不强制顺序时，一个人每隔 1 到 3 秒保存一次，477 次保存中没有出现陈旧值（95% 上限 0.8%）。并发写入会产生短暂的陈旧读取（占写入的 0.26% 到 0.93%），最多持续 43ms，而且没有任何陈旧值在一轮突发写入之后留存（约 1,100 次中 0 次）。只有在有意把回填放慢到 50ms 时，突发之后才会留下陈旧值：相同条件下一次运行为 6.5%，另一次为 0.17%。这是真实存在的竞态，但不至于作为紧急缺陷上报。
