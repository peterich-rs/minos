# Minos Mobile

Flutter shell for the Minos mobile client.

## Cloudflare Access

The app reads Cloudflare Access service-token headers from Flutter
compile-time environment values. CI should pass GitHub Secrets with
`--dart-define`; local runs can forward shell env vars:

```sh
flutter run \
  --dart-define=CF_ACCESS_CLIENT_ID="$CF_ACCESS_CLIENT_ID" \
  --dart-define=CF_ACCESS_CLIENT_SECRET="$CF_ACCESS_CLIENT_SECRET"
```

These values are injected into the in-memory Rust client at startup/pairing
time. They are not persisted to iOS Keychain; Keychain stores only the Minos
business-layer pairing state (`backend_url`, `device_id`, `device_secret`).
