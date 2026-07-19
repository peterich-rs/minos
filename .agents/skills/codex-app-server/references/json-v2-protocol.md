# JSON v2 Protocol Notes

## Transport

Supported app-server transports:

- `stdio` with `--listen stdio://` or default `codex app-server`: newline-delimited JSON, one message per line.
- `websocket` with `--listen ws://IP:PORT`: experimental and unsupported; one JSON-RPC message per WebSocket text frame.
- `off` with `--listen off`: disables local transport.

For WebSocket mode:

- Prefer loopback such as `ws://127.0.0.1:4500`.
- Configure auth before exposing remotely.
- Supported auth modes include capability token and signed bearer token variants.
- Health probes exist at `/readyz` and `/healthz`.
- Server overload may return JSON-RPC error code `-32001` with message `Server overloaded; retry later.` Use exponential backoff with jitter.

## Message framing

Although app-server uses JSON-RPC 2.0 concepts, the on-wire message omits the `jsonrpc` header.

Request:

```json
{ "method": "thread/start", "id": 10, "params": { "model": "gpt-5.4" } }
```

Successful response:

```json
{ "id": 10, "result": { "thread": { "id": "thr_123" } } }
```

Error response:

```json
{ "id": 10, "error": { "code": 123, "message": "Something went wrong" } }
```

Notification:

```json
{ "method": "turn/started", "params": { "turn": { "id": "turn_456" } } }
```

## Initialization handshake

Every connection must send exactly one `initialize` request, then an `initialized` notification. Requests before initialization receive a `Not initialized` error. Repeated initialize calls on the same connection return `Already initialized`.

Minimal handshake:

```json
{ "method": "initialize", "id": 0, "params": { "clientInfo": { "name": "my_client", "title": "My Client", "version": "0.1.0" } } }
{ "method": "initialized", "params": {} }
```

Use `clientInfo.name` to identify the integration. For enterprise-facing integrations, tell the user this value may matter for compliance logs and should be stable.

## Capabilities

`initialize.params.capabilities` may include:

- `experimentalApi: true` to opt in to experimental methods/fields.
- `optOutNotificationMethods: [...]` to suppress exact notification method names on the connection.

Do not opt into experimental API by default.

## Schema generation

Always generate schemas/types for the installed Codex version:

```bash
codex app-server generate-ts --out ./schemas
codex app-server generate-json-schema --out ./schemas
```

Use generated artifacts to validate exact request/response shapes in production clients.
