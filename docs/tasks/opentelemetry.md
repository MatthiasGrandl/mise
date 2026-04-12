# OpenTelemetry <Badge type="warning" text="experimental" />

mise can export traces and logs for `mise run` to any OpenTelemetry-compatible backend such as
[Jaeger](https://www.jaegertracing.io/), [Grafana Tempo](https://grafana.com/oss/tempo/), or
[SigNoz](https://signoz.io/).

This is useful when you want to answer questions like:

- Which task is slow?
- Which task failed?
- What did a task print to stdout/stderr?
- Which part of a monorepo run did a task belong to?

## Quick Start

Set `otel.endpoint` to your OTLP/HTTP collector endpoint:

```toml [mise.toml]
[settings]
otel.endpoint = "http://localhost:4318"
```

Or with an environment variable:

```bash
export MISE_OTEL_ENDPOINT=http://localhost:4318
```

Then run your tasks as usual:

```bash
mise run build ::: test ::: lint
```

If your collector is reachable, mise will export:

- one trace for the full `mise run`
- spans for individual tasks
- grouped spans for monorepo task roots
- task logs from stdout/stderr

### Settings

| Setting | Env Var | Default | Description |
| --- | --- | --- | --- |
| `otel.endpoint` | `MISE_OTEL_ENDPOINT` | _(unset)_ | OTLP collector URL. mise sends traces to `<endpoint>/v1/traces` and logs to `<endpoint>/v1/logs`. |
| `otel.service_name` | `MISE_OTEL_SERVICE_NAME` | `mise` | The `service.name` resource attribute on exported telemetry. |
| `otel.headers` | `MISE_OTEL_HEADERS` | _(unset)_ | Extra headers to send with export requests, for example authentication headers. |

Example with custom headers:

```toml [mise.toml]
[settings.otel.headers]
Authorization = "Bearer mytoken"
```

## What You See

Each `mise run` creates one trace.

That trace contains:

- a root span for the full `mise run`
- task spans for individual tasks
- monorepo group spans when tasks come from different `config_root`s

Typical shape:

```
mise run                          ← root span (full duration)
├── packages/frontend             ← monorepo group span
│   ├── lint                      ← task span
│   ├── typecheck                 ← task span
│   └── build                     ← task span
├── packages/backend              ← monorepo group span
│   └── test                      ← task span
└── deploy                        ← task span (direct child of root)
```

For monorepos, this makes it easier to see which package or subproject a task came from. See
[Monorepo Tasks](/tasks/monorepo) for background on `config_root`.

Task spans include attributes such as:

| Attribute | Description |
| --- | --- |
| `mise.task.name` | Task name |
| `mise.task.source` | Path to the config file defining the task |
| `mise.task.config_root` | Config root directory (for monorepo tasks) |
| `mise.task.skipped` | `"true"` when the task was skipped because sources were up to date |

## Logs

Task stdout and stderr are exported as logs and linked to the corresponding task span, so you can
inspect output directly from the trace.

- stdout is exported with severity `INFO`
- stderr is exported with severity `ERROR`

::: tip
Log streaming works with output modes that capture lines: `prefix`, `keep-order`, and `timed`.
In `interleave` and `raw` mode, output goes directly to the terminal and is not exported as logs.
:::

## Example: Local Development with Jaeger

Start Jaeger with OTLP/HTTP support:

```bash
docker run -d --name jaeger \
  -p 16686:16686 \
  -p 4318:4318 \
  jaegertracing/all-in-one:latest
```

Configure mise:

```toml [mise.toml]
[settings]
otel.endpoint = "http://localhost:4318"
```

Now run any mise task and open `http://localhost:16686`.

## Notes

- When `otel.endpoint` is not set, mise does not create trace context or export any telemetry.
- Export failures are logged at debug level and never break task execution.
- No additional dependencies are required — mise uses its existing HTTP client.
