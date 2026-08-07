# Media blobs (Cloudflare R2)

Minos backend stores **attachment metadata** in Postgres/SQLite (`media_blobs`) and **bytes** in object storage:

| Backend | When |
|---------|------|
| **Cloudflare R2** | `MINOS_R2_ACCOUNT_ID` + access key + secret + bucket are all set |
| **Local directory** | R2 unset and `MINOS_MEDIA_LOCAL_DIR` is set (dev only) |
| **Disabled** | Neither configured → `/v1/media/*` mutating calls return `503 media_not_configured` |

Object storage keeps large files off the small VPS disk. The API process only buffers upload bytes in memory for the request.

## Enable R2 (one-time) — where to click

Your account returns **“Please enable R2 through the Cloudflare Dashboard”** until this is done once.

### Open the R2 overview

1. Log in: [https://dash.cloudflare.com](https://dash.cloudflare.com)
2. Select the **account** (top-left account switcher if you have several).
3. In the left sidebar open **R2** (sometimes under **Storage & databases** → **R2**).
   - Deep link (after login): [https://dash.cloudflare.com/?to=/:account/r2/overview](https://dash.cloudflare.com/?to=/:account/r2/overview)

### First-time enable / purchase free tier

4. On first visit Cloudflare shows a **Get started** / **Purchase R2** / **Enable R2** screen.
   - Free tier (≈10 GB + ops) still requires accepting R2 on the account (billing profile may be required even at $0).
5. Accept / enable R2. Wait until the **Overview** page shows **Create bucket** (not the enable wall).

### Create bucket + API token

6. **Create bucket** → name e.g. `minos-media` → create (keep private; no public access needed).
7. On R2 Overview, under **Account details**, open **Manage R2 API Tokens** (or **Manage** next to API Tokens).
   - Prefer **Create Account API token** with **Object Read & Write**, scoped to `minos-media` if offered.
8. Copy **Access Key ID** and **Secret Access Key** (secret is shown only once).
9. Account ID is on the R2 Overview **Account details** card (also used in endpoint `https://<ACCOUNT_ID>.r2.cloudflarestorage.com`).

Official token docs: [R2 Authentication](https://developers.cloudflare.com/r2/api/tokens/).

## Configure production VPS

In `/opt/minos/deploy/minos.env` (see `deploy/prod/minos.env.example`; `.env` is a symlink):

```bash
MINOS_R2_ACCOUNT_ID=...
MINOS_R2_ACCESS_KEY_ID=...
MINOS_R2_SECRET_ACCESS_KEY=...
MINOS_R2_BUCKET=minos-media
# optional:
# MINOS_R2_ENDPOINT=https://<accountid>.r2.cloudflarestorage.com
# MINOS_MEDIA_MAX_BYTES=10485760
# MINOS_MEDIA_PUBLIC_BASE_URL=https://minos.ainexc.com
```

Restart backend:

```bash
cd /opt/minos/deploy && docker compose up -d minos-backend
# or systemd binary path: systemctl restart minos-backend
```

Check:

```bash
curl -sS https://minos.ainexc.com/v1/media/status
# {"configured":true,"backend":"r2","max_bytes":10485760}
```

## Message + Agent flow (end-to-end)

1. Client uploads: `POST /v1/media/blobs` → `PUT …/content` → `blob_id` **ready**.
2. Client sends chat: `POST /v1/conversations/:id/messages` with  
   `{ "text": "…", "attachment_blob_ids": ["…"] }`  
   (text may be empty if only attachments).
3. Hub stores `chat_message_attachments` and fans out `ChatMessageSummary.attachments`.
4. On `@agent` dispatch, backend signs short-lived download URLs and puts them on the Host command as `attachments[]`.
5. **Host daemon** downloads into  
   `{workspace}/.minos/attachments/{origin_message_id}/…`  
   and appends `@/abs/path` lines to the agent prompt (Grok-style path attach; Codex can later use `localImage`).

**Production requirement for step 4–5:** set `MINOS_MEDIA_PUBLIC_BASE_URL=https://minos.ainexc.com` on the **server** (absolute download URLs). Host may also set `MINOS_BACKEND_URL` if URLs are still relative.

## HTTP API (account bearer)

| Method | Path | Purpose |
|--------|------|---------|
| `GET` | `/v1/media/status` | Configured? backend name, max size (public) |
| `POST` | `/v1/media/blobs` | Create pending blob `{ content_type, byte_size, original_filename? }` |
| `PUT` | `/v1/media/blobs/:blob_id/content` | Upload raw body (must match declared size + type) |
| `POST` | `/v1/media/blobs/get` | `{ blob_id }` → metadata + short-lived `download_url` |
| `GET` | `/v1/media/blobs/:blob_id/content` | Stream bytes (bearer **or** `?token=` from get) |
| `POST` | `/v1/media/blobs/delete` | Soft-delete metadata + delete object |

### Example

```bash
# 1) declare
curl -sS -X POST "$BASE/v1/media/blobs" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"content_type":"image/png","byte_size":1234,"original_filename":"shot.png"}'
# → blob_id, upload_path

# 2) put bytes
curl -sS -X PUT "$BASE/v1/media/blobs/$BLOB_ID/content" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: image/png" \
  --data-binary @shot.png

# 3) download URL
curl -sS -X POST "$BASE/v1/media/blobs/get" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d "{\"blob_id\":\"$BLOB_ID\"}"
```

## Client expectations (compression)

Clients **should** resize/re-encode images (WebP/JPEG, long edge ≤ 1920–2560) before upload. Server enforces MIME allowlist + `MINOS_MEDIA_MAX_BYTES` (default 10 MiB). Gzip of already-compressed images is not required.

## Local dev without R2

```bash
export MINOS_MEDIA_LOCAL_DIR=/tmp/minos-media
mkdir -p "$MINOS_MEDIA_LOCAL_DIR"
# run backend as usual
```

## Free tier note

R2 free allowance is roughly **10 GB storage**, **1M Class A**, **10M Class B**, **egress free**. Deletes are free. Monitor usage in the Cloudflare dashboard.

## Security

- Bucket stays private; downloads go through Minos (`?token=` HMAC with `MINOS_JWT_SECRET`).
- Object keys are `accounts/{account_id}/{kind}/{blob_id}.ext`.
- Path traversal is rejected on the local backend; R2 keys are server-generated only.
