# 故障排查

## NOT_REACHED

```text
run   1  NOT_REACHED
          sync point `refill:after-read` not reached within 10000ms
```

被启用的调用没有到达，所以强制的顺序没有发生。请依次检查：

1. **应用中的钩子是否开启。** `CAWIT_SYNC_URL` 必须设置在应用的环境中（而不只是执行器的环境），在 `NODE_ENV=production` 下还需要 `CAWIT_ALLOW_PRODUCTION=1`。为了确认，可以在启动时打印 `require("cawit-hook").enabled`。
2. **是否是同一个 Redis。** 应用的 `CAWIT_SYNC_URL` 和场景的 `redis` 必须指向同一个 Redis 服务器和同一个数据库编号。
3. **请求是否经过了你加钩子的路径。** 命中缓存的读取永远不会到达回填钩子：模式的第一次写入会使缓存项失效，但 Redis 前面的其他缓存（进程内 LRU、HTTP 缓存）仍可能直接响应。查看应用日志，确认发生了数据库读取。
4. **键是否匹配。** 如果场景以 `key: X` 启用，钩子必须恰好以 `X` 调用。框架看到的路径常常与客户端发送的不同：在 Express 中，挂载路由内部的 `req.path` 不含挂载前缀（请使用 `req.baseUrl + req.path`）。
5. **构建产物里是否是当前的钩子。** 打包工具可能把旧版 `cawit-hook` 打进了构建。在一轮运行时观察 Redis：

   ```bash
   redis-cli monitor | grep -E 'cawit:|EVAL'
   ```

   你应该依次看到执行器的 `SET cawit:arm:<point>`、钩子的 `EVAL`，以及 `RPUSH cawit:reached:<token>`。如果钩子的 `EVAL` 脚本与当前版本不同，请重新构建。

## "the hold at … timed out before release"

被暂停的调用在场景放行之前就停止了等待，顺序被破坏，该轮被丢弃。要么场景在 `reached` 和 `release` 之间等待的时间超过了钩子的 `CAWIT_SYNC_TIMEOUT_S`（默认 30 秒），要么中间某个步骤（HTTP 请求）花了这么久。请在应用环境中调大超时，或找出慢的步骤。

## ERROR

```text
run   1  ERROR
          step 6 (http): HTTP 500 from http://127.0.0.1:3001/items/articles/1: …
```

某个请求返回了错误状态（或在设置了 `error_field` 时返回了 GraphQL `errors` 字段）、某个检查无法读取它的值，或者某个步骤的配置有误。备注会指出是哪一步。如果该请求预期会失败（故障注入），请在那个 `http` 步骤上设置 `allow_error: true`；`failed-invalidation` 模式会替你这样做。

## 还没修复就全部通过

- 对于这种顺序，竞态可能确实是关闭的（例如代码在提交后才失效，而模式是 `invalidate-before-commit`）。
- 或者场景没有走到那条路径：用 `--show-steps` 检查，确认读取确实未命中并回填（应用日志），并确认 `state` 比较的是正确的值。使用 `app_view` 时，设置 `hit_header` 可以让 `refilled` 判定证明该值来自缓存。

## 各轮结果不一致

在场景控制的顺序和等待之内，各轮是确定的。如果结果不一致：

- 被测代码很可能依赖定时器；用 `hold_ms` 固定时间，或使用 `within_ms`（[竞态模式](patterns.md#时间参数)）；
- 启动后的第一轮较慢（冷缓存、JIT、连接池）；
- 场景与同时运行的其他东西共享数据库或缓存。

## "unknown variable ${…}" 或 "environment variable … is not set"

模板会明确报错，而不是把 `${...}` 原样发送出去。请在步骤的 `vars` 中定义变量、使用 `${run}`/`${run_index}`，或为 `cawit` 进程导出该环境变量。

## "unknown field …"

场景文件在每一层都拒绝未知的键，所以拼写错误会在加载时失败。请对照[场景格式](../reference/scenario-format.md)。
