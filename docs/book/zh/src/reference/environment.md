# 环境变量

## 应用中（钩子）

| 变量 | 默认值 | 含义 |
|---|---|---|
| `CAWIT_SYNC_URL` | 未设置 | 钩子协议使用的 Redis URL，`redis://host[:port][/db]`。未设置时钩子关闭且不做任何 I/O |
| `CAWIT_SYNC_TIMEOUT_S` | `30` | 被暂停的调用等待放行的秒数。暂停过期会使该轮为 `NOT_REACHED` |
| `CAWIT_ALLOW_PRODUCTION` | 未设置 | 在 `NODE_ENV=production` 下，除非此值为 `1`，否则钩子保持关闭 |
| `CAWIT_SYNC_TLS_INSECURE` | 未设置 | `1`：不验证 `rediss://` 服务器的证书（仅限测试服务器） |
| `NODE_EXTRA_CA_CERTS` | 未设置 | Node 的标准变量：额外信任的 CA（PEM），钩子在 `rediss://` 中使用 |

## 执行器中

`cawit` 本身不读取任何环境变量。场景可以在请求模板以及 `apps`、`redis`、`postgres` 中用 `${env.NAME}` 读取变量（请用于密码）；未设置的变量会报错。
