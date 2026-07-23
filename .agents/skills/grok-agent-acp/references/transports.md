# Grok Agent Transports

## 1. stdio — Minos default

```bash
grok agent --no-leader stdio
```

- Parent owns process lifecycle (`kill_on_drop`, process group on Unix).
- One JSON-RPC line per message.
- Best for embedded TUI/daemon: crash of parent kills agent.

**Why `--no-leader`:** without it, Grok may connect to a shared leader and surprise-share state with other clients on the machine.

## 2. serve — WebSocket server (Codex app-server-like)

```bash
export GROK_AGENT_SECRET="$(openssl rand -hex 16)"
grok agent serve --bind 127.0.0.1:2419 --secret "$GROK_AGENT_SECRET"
```

- Long-lived process.
- Clients authenticate with secret.
- Good for multi-surface UIs that should not respawn the model runtime per client.

Security:

- Bind loopback unless you have auth/TLS in front.
- Rotate secrets; do not log them.
- Prefer OS keychain / env injection over config files in shared homes.

## 3. leader — shared UDS backend

Default socket: `~/.grok/leader.sock`  
Override: `--leader-socket PATH` or `GROK_LEADER_SOCKET`.

Commands:

```bash
grok leader list
grok leader info
grok leader kill
grok agent leader --no-exit-on-disconnect
```

Use when multiple Grok clients intentionally share one backend. **Not** the primary Minos integration surface; Minos speaks ACP stdio per agent session.

## 4. headless streaming-json (not ACP)

```bash
grok -p "…" --output-format streaming-json
```

Coarse NDJSON (`text` / `thought` / `end`). Insufficient for tool/permission UX. Prefer ACP for product integration.

## Choosing a transport

| Need | Choose |
|------|--------|
| Minos TUI thread per workspace | stdio + `--no-leader` |
| External IDE plugin sharing process | serve or leader |
| Browser over internet | headless relay / your own bridge in front of serve |
| CI one-shot scripts | `-p` headless (or ACP if tool stream required) |
