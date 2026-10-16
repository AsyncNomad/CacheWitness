# Environment Variables

## In the application (hook)

| Variable | Default | Meaning |
|---|---|---|
| `CAWIT_SYNC_URL` | unset | Redis URL for the hook protocol, `redis://host[:port][/db]`. Unset: the hook is off and does no I/O |
| `CAWIT_SYNC_TIMEOUT_S` | `30` | Seconds a held call waits for release before continuing. A hold that expires makes the run `NOT_REACHED` |
| `CAWIT_ALLOW_PRODUCTION` | unset | With `NODE_ENV=production`, the hook stays off unless this is `1` |

## In the runner

`cawit` itself reads no environment variables. Scenarios can read any variable with `${env.NAME}` in request templates; an unset variable is an error.
