# 场景格式

场景是一个 YAML 文件。每一层的未知键都是错误，所以拼写错误会在加载时失败，而不会被忽略。

```yaml
name: directus-stale-set          # 必填
runs: 20                          # 默认 1；--runs 优先
apps:                             # 必填：具名的基础 URL（仅 http://）
  a: http://127.0.0.1:8055
  b: http://127.0.0.1:8056
redis: redis://127.0.0.1:6379     # 必填：协议 + 缓存读取
postgres: "host=127.0.0.1 user=postgres password=postgres dbname=directus"

requests: { ... }                 # 具名请求模板
state: { ... }                    # 检查所比较的内容
pattern: { ... }                  # 要么是模式……
# steps: [ ... ]                  # ……要么是手写步骤
```

## 顶层

| 键 | 类型 | 必填 | 含义 |
|---|---|---|---|
| `name` | 字符串 | 是 | 输出标签和 JUnit 套件名 |
| `runs` | 整数 | 否（1） | 轮数 |
| `apps` | 名称 → URL | 是 | 应用服务器，`http://` 或 `https://` |
| `redis` | 字符串 | 是 | 用于钩子协议和 `cache_key` 读取的 `redis://[[user]:password@]host[:port][/db]`，TLS 用 `rediss://`；必须是应用 `CAWIT_SYNC_URL` 指向的 Redis |
| `postgres` | 字符串 | 检查需要 | libpq key=value 连接字符串；遵循 `sslmode`（[TLS](#tls-与认证)） |
| `tls` | 映射 | 否 | 用于 `rediss://` 和 `https://` 的 TLS 选项（[见下](#tls-与认证)） |
| `requests` | 名称 → [请求](#请求) | 实际上必填 | 请求模板 |
| `state` | [状态](#状态) | 检查需要 | 检查所比较的内容 |
| `pattern` | 单键映射 | 二选一 | 一个[模式](../guide/patterns.md)及其参数 |
| `steps` | 列表 | 二选一 | 手写的[步骤](#步骤) |

`pattern` 和 `steps` 必须恰好出现一个。

## TLS 与认证

| 连接 | 方式 |
|---|---|
| Redis | `redis://user:password@host`（或 `:password@`，或只有 `password@`）会发送 `AUTH`；`rediss://` 使用 TLS。用户名和密码会做百分号解码 |
| 应用 | 在 `apps` 中使用 `https://` URL |
| PostgreSQL | 连接字符串中的 `sslmode`：`disable`、`prefer`（默认：服务器支持时使用 TLS）、`require`。与 libpq 在这些模式下一致，不验证服务器证书 |

Redis 和应用的证书会用系统信任库验证（为空时使用内置的 Mozilla 根证书）。对于私有 CA 或自签名的测试服务器：

```yaml
tls:
  ca_file: certs/ca.pem    # 额外信任的 CA（PEM），相对于场景文件
  insecure: false          # true：不验证证书（仅限测试服务器）
```

不要把密码写进场景文件：`redis`、`postgres` 和 `apps` 中的 URL 都支持 `${env.X}`，例如 `redis: rediss://:${env.REDIS_PASSWORD}@cache:6380`。所有输出中 Redis URL 的密码都显示为 `***`。

## 请求

| 键 | 类型 | 默认值 | 含义 |
|---|---|---|---|
| `method` | 字符串 | `POST` | HTTP 方法 |
| `path` | 字符串 | 必填 | 路径和查询 |
| `headers` | 映射 | `{}` | 额外的请求头 |
| `json` | 任意 | 无 | JSON 请求体，以 `content-type: application/json` 发送 |
| `error_field` | 字符串 | 无 | 如果响应 JSON 中这个顶层字段的值不为 null，就视为错误（GraphQL：`errors`） |

状态码 400 及以上是错误。除非步骤设置了 `allow_error`，错误会让该轮以 `ERROR` 结束。

### 变量

`path`、请求头的值以及 `json` 中的每个字符串都可以包含 `${...}`：

| 变量 | 值 |
|---|---|
| `${run}` | 每轮唯一：一次运行中所有轮次共享的运行 ID、一个短横线和轮次编号 |
| `${run_index}` | 轮次编号，从 1 开始 |
| `${env.NAME}` | `cawit` 进程的环境变量；未设置时报错 |
| `${name}` | 步骤 `vars` 中的一项（其值本身也可以使用 `${run}` 和 `${env.X}`） |

未知变量会报错。`arm` 步骤的 `key` 也以同样方式渲染。`redis`、`postgres` 和 `apps` 可以使用 `${env.X}`；`state` 中的值按原样使用。

## 状态

| 键 | 类型 | 含义 |
|---|---|---|
| `db_query` | 字符串 | 在第一行第一列以文本返回已提交值的 SQL（用 `::text` 转换） |
| `cache_key` | 字符串 | 缓存项的 Redis 键 |
| `cache_field` | 字符串 | 当缓存项是 JSON 对象时要取出的顶层字段 |
| `app_view` | 映射 | 改为通过应用读取（见下） |

`app_view`：

| 键 | 默认值 | 含义 |
|---|---|---|
| `app` | 必填 | 发送请求的应用 |
| `request` | 必填 | 请求模板 |
| `field` | 必填 | 响应体中的 JSON pointer，例如 `/data/title` |
| `hit_header` | 无 | 表明响应是否来自缓存的响应头 |
| `hit_value` | `HIT` | `hit_header` 中表示命中的值 |

## 模式

见[竞态模式](../guide/patterns.md)。参数：

| 参数 | `stale-set` | `failed-invalidation` | `invalidate-before-commit` | 默认值 |
|---|---|---|---|---|
| `reader` | 必填 | | 必填 | |
| `writer` | 必填 | | 必填 | |
| `app` | | 必填 | | |
| `write`、`read` | 必填 | 必填 | 必填 | |
| `value_var` | ✓ | ✓ | ✓ | `value` |
| `point` | ✓ | ✓ | ✓ | `refill:after-read` / `invalidate:before` / `write:before-commit` |
| `key` | ✓ | ✓ | ✓ | 任意键 |
| `hold_ms` | ✓ | | ✓ | `0` |
| `settle_ms` | ✓ | ✓ | ✓ | `100` |
| `within_ms` | ✓ | ✓ | ✓ | 严格 |
| `reached_timeout_ms` | ✓ | ✓ | ✓ | `10000` |

## 步骤

每个步骤是一个单键映射：`{ <类型>: { <参数> } }`。

### `http`

| 参数 | 默认值 | 含义 |
|---|---|---|
| `app` | 必填 | 应用名 |
| `request` | 必填 | 请求模板名 |
| `vars` | `{}` | 额外的模板变量 |
| `async` | 无 | 以该任务名在后台运行；用 `await` 收回 |
| `allow_error` | `false` | 错误响应不会结束该轮 |

### `arm`

| 参数 | 默认值 | 含义 |
|---|---|---|
| `point` | 必填 | 同步点名称 |
| `key` | 任意键 | 只取走带这个键的调用 |
| `action` | `hold` | `hold`（等待放行）或 `fail`（抛出一次异常） |
| `as` | 必填 | 供 `reached` 和 `release` 使用的别名 |

如果没有调用取走，启用会在 30 秒后过期。

### `reached`

| 参数 | 默认值 | 含义 |
|---|---|---|
| `sync` | 必填 | 别名 |
| `timeout_ms` | `10000` | 等待多久；超时后该轮为 `NOT_REACHED` |

### `release`

| 参数 | 含义 |
|---|---|
| `sync` | 别名。如果应用中的暂停已经超时，该轮为 `NOT_REACHED` |

### `await`、`sleep`

`await: { task: <名称> }` 等待一个后台请求（除非它允许错误，否则其错误会结束该轮）。`sleep: { ms: <n> }` 等待。

### `expect`

| 参数 | 默认值 | 含义 |
|---|---|---|
| `name` | 必填 | 输出和报告中的标签 |
| `check` | 必填 | 一个检查（见下） |
| `within_ms` | 严格 | 每 50ms 重试一次；在该时间内通过即为通过 |

## 检查

| 检查 | 需要 | 通过条件 |
|---|---|---|
| `cache_not_stale` | `cache_key`、`postgres` | 缓存项不存在，或等于数据库中的值 |
| `cache_equals_db` | `cache_key`、`postgres` | 缓存项存在且等于数据库中的值 |
| `cache_absent` | `cache_key` | 缓存项不存在 |
| `app_equals_db` | `app_view`、`postgres` | 应用返回数据库中的值 |
| `app_cached_equals_db` | 带 `hit_header` 的 `app_view`、`postgres` | 同上，并且响应命中了缓存 |

## 一轮的结束

无论发生了什么，每一轮结束时都会取消该轮启用的同步点、放行仍被暂停的调用，并最多等待 10 秒让后台请求结束，因此一轮绝不会让应用保持阻塞而影响下一轮。

## 手写示例

把 `stale-set` 模式完整写出来：

```yaml
steps:
  - http: { app: a, request: update, vars: { value: "${run}-a" } }
  - sleep: { ms: 50 }
  - arm: { point: "refill:after-read", key: /items/articles/1, as: refill }
  - http: { app: a, request: read, async: held_read }
  - reached: { sync: refill, timeout_ms: 10000 }
  - http: { app: b, request: update, vars: { value: "${run}-b" } }
  - release: { sync: refill }
  - await: { task: held_read }
  - sleep: { ms: 100 }
  - expect: { name: not_stale, check: app_equals_db }
  - expect: { name: refilled, check: app_cached_equals_db }
```
