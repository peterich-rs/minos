# Cloudflare Tunnel + Access Setup for `minos-backend`

Operational runbook for bringing up the public ingress in front of `minos-backend`. Source of truth for the architectural choices is ADR 0010 (tunnel topology) plus ADR 0014 (CF Access tokens now live in the backend's env vars and are distributed to mobile via the pairing QR, not the mac app's keychain). This document captures the concrete commands to reproduce the tunnel on a fresh machine.

**Scope:** one named tunnel (`minos`) bound to one hostname (`minos.fan-nn.top`), forwarding to `http://127.0.0.1:8787` (the backend). A Cloudflare Access application gates the hostname for both interactive browser access and Service-Token-authenticated clients (the iOS app — agent-host talks to backend over loopback and never crosses the CF edge).

**Prerequisites:**

- A Cloudflare account with the domain (`fan-nn.top`) already on Cloudflare name servers.
- Admin access to the Cloudflare Zero Trust dashboard.
- A Mac or Linux box where the backend will run (MVP: the owner's Mac).
- Homebrew installed on macOS, or equivalent package manager on Linux.

Commands below use `<UUID>` / `<CLIENT_ID>` / `<CLIENT_SECRET>` as placeholders. Never commit the real values.

---

## 1. Install `cloudflared`

macOS:

```bash
brew install cloudflared
```

Linux (Debian / Ubuntu):

```bash
curl -L https://pkg.cloudflare.com/install | sudo bash
sudo apt-get install cloudflared
```

Verify:

```bash
cloudflared --version
```

---

## 2. Authenticate cloudflared to your Cloudflare account

```bash
cloudflared tunnel login
```

A browser window opens. Select the zone for your domain. On success, the account-level certificate lands at:

```
~/.cloudflared/cert.pem
```

This file identifies the *account* that may create tunnels; treat it like an SSH private key. It is not the tunnel's own credential.

---

## 3. Create the tunnel

```bash
cloudflared tunnel create minos
```

Cloudflare generates a tunnel UUID and writes per-tunnel credentials to:

```
~/.cloudflared/<UUID>.json
```

Write the UUID down; it appears again in the config file.

---

## 4. Route DNS to the tunnel

```bash
cloudflared tunnel route dns minos minos.fan-nn.top
```

This creates a proxied CNAME `minos → <UUID>.cfargotunnel.com` in the Cloudflare DNS panel without manual clicking. Replace `minos.fan-nn.top` with your actual subdomain.

---

## 5. Write the tunnel config

File: `~/.cloudflared/config.yml`

```yaml
tunnel: <UUID>
credentials-file: /Users/<you>/.cloudflared/<UUID>.json

ingress:
  - hostname: minos.fan-nn.top
    service: http://localhost:8787
    originRequest:
      connectTimeout: 10s
      keepAliveTimeout: 90s
      noHappyEyeballs: true
  - service: http_status:404
```

Notes:

- `service` points at the backend's loopback listen address (`127.0.0.1:8787` by default).
- `keepAliveTimeout: 90s` pairs with the backend's heartbeat; clients send application-layer ping every ~30s to avoid CF edge's idle cutoff.
- The trailing `http_status:404` is required by cloudflared as a catch-all for unmatched hostnames.
- The final `service:` value must be a URL or pseudo-URL (`http_status:N`, `hello_world`, etc.); `config.yml` refuses a bare service entry.

---

## 6. Smoke-test the tunnel before installing as a service

Start the backend (or any temporary server on port 8787):

```bash
# If the backend exists:
cargo xtask backend-run

# Or temporarily:
python3 -m http.server 8787
```

In another terminal, run the tunnel in foreground:

```bash
cloudflared tunnel run minos
```

In a third terminal, verify the public hostname resolves and reaches the origin:

```bash
curl -v https://minos.fan-nn.top/
```

Expect the origin's response (backend's 404 for `/`, or the Python server's directory listing).

**If this fails:**
- Check `cloudflared` logs for the tunnel handshake.
- Confirm the CNAME exists in the Cloudflare DNS dashboard.
- Confirm `ingress.service` points at the correct local port.
- Check no local firewall is blocking loopback (unusual on macOS).

Stop both when verified (`Ctrl-C`).

---

## 7. Install cloudflared as a system service

macOS (installs a LaunchDaemon, starts on boot):

```bash
sudo cloudflared service install
```

Verify:

```bash
sudo launchctl list | grep cloudflared
```

Tail the live log stream:

```bash
sudo log stream --predicate 'subsystem == "com.cloudflare.cloudflared"'
```

Uninstall (if needed):

```bash
sudo cloudflared service uninstall
```

Linux (systemd):

```bash
sudo cloudflared service install
sudo systemctl status cloudflared
sudo journalctl -u cloudflared -f
```

### 7a. Backend service env (no public-URL or CF-Access vars)

The backend itself does not need `MINOS_BACKEND_URL`,
`CF_ACCESS_CLIENT_ID`, or `CF_ACCESS_CLIENT_SECRET`. Mobile and daemon
clients dial the URL baked at build time (set via `.env.local`,
documented in
`docs/superpowers/specs/unified-config-pipeline-design.md`). Cloudflare
Access service tokens are configured on clients only — the backend is
unaware of CF Access (it sees post-edge HTTP loopback on
`127.0.0.1:8787`).

The backend does still need `MINOS_JWT_SECRET` (account-auth bearer
token signing). Set it in the LaunchDaemon plist or systemd drop-in
the same way you'd set any other env var; the backend panics at boot
if absent or shorter than 32 bytes.

---

## 8. Create a Cloudflare Access application

The tunnel is now publicly reachable. Without Access, any client that knows the URL can reach the backend. Put the hostname behind Access next.

Dashboard path:

```
Cloudflare Dashboard → Zero Trust → Access → Applications → Add an application → Self-hosted
```

Fill in:

- **Application name**: `Minos Backend`
- **Session duration**: `24h` (or whatever the team prefers; shorter is stricter)
- **Application domain**: `minos.fan-nn.top`
- **Policy name**: `Owner access`
- **Action**: `Allow`
- **Include**: `Emails` → your owner email address(es)

Save. The hostname is now gated.

**Verify the gate:** visiting `https://minos.fan-nn.top/` in an incognito browser should redirect to a Cloudflare Access sign-in page.

---

## 9. Mint a Service Token for non-browser clients

The iOS app cannot complete an interactive SSO flow. Generate a machine credential and have the **backend** carry it (per ADR 0014); mobile picks it up via the pairing QR.

Dashboard path:

```
Cloudflare Dashboard → Zero Trust → Access → Service Auth → Service Tokens → Create Service Token
```

Fill in:

- **Service Token name**: `minos-mobile`
- **Duration**: longest available (you will rotate manually if compromised)

On save, Cloudflare shows the `Client ID` and `Client Secret` **once**. Paste them into the backend's env vars (step 7a) and restart the backend service.

Go back to the Access application you created in step 8, edit its policy, and add a second rule:

- **Action**: `Service Auth`
- **Include**: `Service Token` → `minos-mobile`

Save. The token is now authorized for this hostname.

---

## 10. Verify Service Token works

```bash
curl -v \
  -H "CF-Access-Client-Id: <CLIENT_ID>.access" \
  -H "CF-Access-Client-Secret: <CLIENT_SECRET>" \
  https://minos.fan-nn.top/health
```

Expect `200 OK` from the backend. If you get `302` or a sign-in page, the headers are not set correctly or the Service Token policy is missing.

---

## 11. Distribute the Service Token to clients

Cloudflare Access is a shared edge gate, not Minos business authorization. Minos still authenticates devices with pairing tokens and per-device `device_secret`.

Agent hosts read the service token from their process environment:

```bash
CF_ACCESS_CLIENT_ID=<CLIENT_ID>.access \
CF_ACCESS_CLIENT_SECRET=<CLIENT_SECRET> \
minos-daemon start
```

The macOS GUI follows the same rule. If launching from `launchd`, set those keys in the app/service environment rather than asking the user to paste them into Keychain.

Mobile reads the token from Flutter compile-time environment values:

```bash
flutter build ios \
  --dart-define=CF_ACCESS_CLIENT_ID="$CF_ACCESS_CLIENT_ID" \
  --dart-define=CF_ACCESS_CLIENT_SECRET="$CF_ACCESS_CLIENT_SECRET"
```

In CI, source those values from GitHub Secrets. The mobile app injects them into outbound REST/WSS requests at runtime and does not persist them to Keychain.

Rotating the token means minting a new Service Token in Cloudflare, updating build/host env configuration, shipping/restarting clients, then revoking the old token after overlap.

**Do not** commit tokens to the repository or paste them into CI configs checked into git. They are long-lived and high-privilege.

---

## 12. Rotation and revocation

| Action | Dashboard path | Effect |
|---|---|---|
| Rotate a Service Token | Zero Trust → Access → Service Auth → Service Tokens → ⋯ → Regenerate | Old secret invalidated; update backend env vars (step 7a) and re-issue the pairing QR for every paired phone |
| Revoke a specific client's access | Remove user's email from the Allow policy (step 8) | Interactive access blocked; Service Token still works until rotated |
| Take the hostname offline | Disable the Application in Access, or stop `cloudflared` | Nothing reaches the backend |

---

## Troubleshooting cheatsheet

| Symptom | Likely cause | Fix |
|---|---|---|
| `curl https://minos.fan-nn.top/health` returns `Cloudflare 1033` | Tunnel not running | Start `cloudflared` or check service status |
| Handshake returns `5xx`  | Backend not listening on configured port | Start `minos-backend`; verify `MINOS_BACKEND_LISTEN` matches `config.yml` `service:` |
| Backend rejects boot with `CfAccessMisconfigured` | `PUBLIC_URL` is `wss://...` but env vars not set | Step 7a; or set `MINOS_BACKEND_ALLOW_DEV=1` for loopback dev |
| Clients disconnect every ~100s | Heartbeat not firing | Check `minos-transport` heartbeat loop; CF edge idle cutoff is ~100s |
| `curl` with Service Token returns sign-in page | Service Token policy missing from Access application | Add Service Auth rule in step 9 revision |
| New dev machine: `cloudflared tunnel run minos` fails with "credentials not found" | `cert.pem` / `<UUID>.json` not present on this machine | Re-run `cloudflared tunnel login` + copy the credential JSON from the original machine |

---

## Files that belong in git versus files that don't

**Never commit:**

- `~/.cloudflared/cert.pem` — account-level Cloudflare credential.
- `~/.cloudflared/<UUID>.json` — tunnel-specific credential.
- Service Token `Client ID` / `Client Secret` — in any form, including example configs.

**Safe to commit:**

- `~/.cloudflared/config.yml` template with `<UUID>` placeholder in place of the real tunnel UUID.
- This runbook.
- Application configuration that references environment variables for tokens, never the values themselves.

---

## Reference

- ADR 0010: Cloudflare Tunnel + Access for Public Exposure
- ADR 0014: Backend-Assembled Pairing QR (CF Access tokens leave the host)
- ADR 0015: Rename `minos-relay` → `minos-backend`
- Spec: `docs/superpowers/specs/minos-relay-backend-design.md` §4.3 (security boundaries) and §9.3 (runbook reference) — note: filename retains the historical `minos-relay-backend-design` slug, see ADR 0015
