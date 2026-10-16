# Environment Variables

## In the application (hook)

| Variable | Default | Meaning |
|---|---|---|
| `CAWIT_SYNC_URL` | unset | Redis URL for the hook protocol, `redis://host[:port][/db]`. Unset: the hook is off and does no I/O |
| `CAWIT_SYNC_TIMEOUT_S` | `30` | Seconds a held call waits for release before continuing. A hold that expires makes the run `NOT_REACHED` |
| `CAWIT_ALLOW_PRODUCTION` | unset | With `NODE_ENV=production`, the hook stays off unless this is `1` |
| `CAWIT_SYNC_TLS_INSECURE` | unset | `1`: do not verify the certificate of a `rediss://` server (test servers only) |
| `NODE_EXTRA_CA_CERTS` | unset | Node's standard variable: extra trusted CAs (PEM), used by the hook for `rediss://` |

## In the runner

`cawit` itself reads no environment variables. Scenarios read variables with `${env.NAME}` in request templates and in `apps`, `redis`, and `postgres` (use this for passwords); an unset variable is an error.
