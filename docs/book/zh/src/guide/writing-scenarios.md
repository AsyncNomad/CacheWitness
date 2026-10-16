# 编写场景

本指南从"我们怀疑有一个陈旧缓存问题"出发，一直到得到一个修复前失败、修复后通过的场景。

## 1. 选择竞态

| 症状 | 可能的竞态 | 模式 |
|---|---|---|
| 更新之后，读者有时会一直拿到旧值，直到 TTL 到期 | 在更新之前开始的回填在失效之后才完成 | [`stale-set`](patterns.md#stale-set) |
| 更新请求返回了错误（或超时），但值已经变了，缓存却保留了旧值 | 提交之后的失效失败了 | [`failed-invalidation`](patterns.md#failed-invalidation) |
| 更新之后立刻读到旧值，即使之前没有并发读者 | 在事务内、提交之前就使缓存失效了 | [`invalidate-before-commit`](patterns.md#invalidate-before-commit) |

## 2. 找到钩子的位置

每种模式都需要在代码的某一处放一个同步点：

| 模式 | 同步点（默认名称） | 位置 |
|---|---|---|
| `stale-set` | `refill:after-read` | 未命中路径中，读完数据库之后、写入缓存之前 |
| `failed-invalidation` | `invalidate:before` | 写路径中缓存失效之前 |
| `invalidate-before-commit` | `write:before-commit` | 写事务的末尾、提交之前 |

在代码中搜索缓存写入（`set`、`mset`、`setex`、缓存库的 `set` 或 `wrap`）和失效（`del`、`unlink`、`clear`、按标签或命名空间清除）。框架往往把这些集中在一个服务或中间件里；在案例研究中，钩子分别放在一个缓存服务（Twenty）和一个响应中间件（Directus）中。

选择一个能让场景只暂停它关心的那次回填的**键**。一个读请求常常会回填多个缓存项；如果不指定键就启用，可能会先取走一个无关的回填。请使用场景中可以原样写出的值，例如请求路径或 `<实体>:<id>`。

## 3. 请求

写一个用于写入的请求模板和一个用于读取的请求模板。写请求必须在写入的值处使用 `${value}`（或模式的 `value_var` 指定的其他变量），这样每一轮写入的值都是该轮独有的。

```yaml
requests:
  update:
    method: PATCH
    path: /items/articles/1
    headers: { authorization: "Bearer ${env.API_TOKEN}" }
    json: { title: "${value}" }
  read:
    method: GET
    path: /items/articles/1
    headers: { authorization: "Bearer ${env.API_TOKEN}" }
```

对于 GraphQL，请加上 `error_field: errors`：GraphQL 在出错时也返回状态 200，没有这个设置，失败的变更会看起来像成功。

## 4. 状态：比较什么

`db_query` 在第一行第一列以文本形式返回已提交的值。

然后选择如何读取缓存一侧：

- 当你知道 Redis 键、值是纯文本或 JSON 时，用 **`cache_key`**。要比较一个顶层 JSON 字段，加上 `cache_field`。
- 当键是哈希、值经过压缩或序列化，或者真正重要的是响应本身时，用 **`app_view`**。它通过应用发送请求，并用 JSON pointer 从响应体中取值。如果应用在响应头中报告缓存命中，请加上 `hit_header`；这样 `refilled` 判定还会要求命中。

## 5. 一个实例还是两个

场景可以把竞争的写入发送到第二个实例（`writer: b`），也可以发送到同一个实例（`writer: a`）。两者都可以：每个被暂停的调用都在自己的 Redis 连接上等待，所以不会阻塞同一进程中的其他请求。两个实例更接近生产环境，也能测试跨实例的失效。

## 6. 运行、阅读、迭代

```bash
cawit verify scenario.yaml --show-steps   # 检查展开结果
cawit verify scenario.yaml --runs 3       # 快速尝试
cawit verify scenario.yaml                # 按配置的轮数运行
```

- **修复前全部 `FAIL`：** 很好，竞态复现了。
- **`NOT_REACHED`：** 没有到达钩子，或暂停超时。见[故障排查](troubleshooting.md#not_reached)。
- **还没修复就 `PASS`：** 你的代码可能已经能抵御这种顺序，或者场景没有走到你以为的路径。检查 `--show-steps`，并通过应用日志确认读取确实未命中。

## 7. 保留它

把场景和你的测试一起提交，并在 CI 中运行（[在 CI 中运行](ci.md)）。在案例研究中，一个模式运行 20 轮需要 15 到 30 秒。
