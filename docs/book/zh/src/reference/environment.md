# 环境变量

## 应用中（钩子）

| 变量 | 默认值 | 含义 |
|---|---|---|
| `CAWIT_SYNC_URL` | 未设置 | 钩子协议使用的 Redis URL，`redis://host[:port][/db]`。未设置时钩子关闭且不做任何 I/O |
| `CAWIT_SYNC_TIMEOUT_S` | `30` | 被暂停的调用等待放行的秒数。暂停过期会使该轮为 `NOT_REACHED` |
| `CAWIT_ALLOW_PRODUCTION` | 未设置 | 在 `NODE_ENV=production` 下，除非此值为 `1`，否则钩子保持关闭 |

## 执行器中

`cawit` 本身不读取任何环境变量。场景可以在请求模板中用 `${env.NAME}` 读取任意变量；未设置的变量会报错。
