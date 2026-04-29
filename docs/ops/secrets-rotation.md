# Secrets Rotation Runbook

## Cloudflare Access service token

The mobile app and daemon authenticate to the Cloudflare Access edge
using a service-token pair (`CF_ACCESS_CLIENT_ID`, `CF_ACCESS_CLIENT_SECRET`).
Rotate when:

- The secret has been seen in any committed file or shared chat log.
- A developer with access to `.env.local` leaves the project.
- On a quarterly schedule (good hygiene).

### Procedure

1. **Cloudflare Zero Trust → Access → Service Tokens → Minos token → Rotate.**
   Cloudflare displays a NEW client_secret exactly once; copy it to a
   secure scratchpad immediately. The OLD secret continues to work
   until you revoke it.

2. **Update `.env.local`** for each developer:
   ```
   CF_ACCESS_CLIENT_SECRET=<new value>
   ```
   The client_id rarely changes; only the secret rotates.

3. **Update the GitHub Actions secret** at the repository's Settings →
   Secrets and variables → Actions → `CF_ACCESS_CLIENT_SECRET`.

4. **Wait for the next CI build** of the iOS release artifact (~5 min)
   so production has a binary signed with the new value. Tag a release
   if your deploy pipeline requires it.

5. **Revoke the old secret** in Cloudflare Zero Trust. The overlap
   window must be at least one full CI build cycle to avoid breaking
   already-running mobile sessions whose binary still has the old
   secret baked in.

## Backend JWT secret

`MINOS_JWT_SECRET` signs account-auth bearer tokens. Rotation
invalidates all live sessions (users must log in again). Rotate when:

- The secret has been exposed.
- Quarterly hygiene.

Procedure:

1. Generate: `openssl rand -hex 32`
2. Update GitHub Actions secret `MINOS_JWT_SECRET` (production deploy).
3. Update each developer's `.env.local`.
4. Restart the backend (`just backend`).
5. All existing access tokens become invalid; mobile clients receive
   401s on the next request and re-prompt for login.

There is no overlap-window mechanism — JWT rotation is destructive
to live sessions by design. Coordinate with users if the impact matters.
