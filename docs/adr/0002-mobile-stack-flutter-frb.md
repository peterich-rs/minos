# 0002 · Mobile Stack: Flutter + flutter_rust_bridge

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-04-21 |
| Deciders | fannnzhang |

## Context

The mobile client needs:
- Cross-platform reach (iOS now; Android in P1.5).
- Class-leading performance for streaming chat surfaces (where agent token output drives ~20 chunk/s repaints).
- Reuse of the same Rust core that the Mac daemon runs on.

Two routes were considered seriously:

1. **Twin-native, Telegram-style**: take Telegram's open-source iOS (Swift) and Android (Kotlin) UI components — particularly their streaming-text rendering — and wire them to the Minos Rust core via UniFFI. Maximum native performance.
2. **Flutter UI + Rust core via flutter_rust_bridge** (frb v2). One UI codebase across both platforms.

## Decision

Flutter UI + flutter_rust_bridge v2. A `PlatformView`-based native chat surface is reserved as an escape hatch if profiling later shows Flutter cannot meet the streaming-chat target.

## Consequences

**Positive**
- One UI codebase for iOS and Android (and, optionally, desktop / web later).
- frb v2 maps Rust `async fn`, `Stream`, and `Result<T, E>` directly to Dart `Future`, `Stream`, and typed exceptions. No hand-written FFI shims.
- Skia / Impeller renders chat-grade UI smoothly; `shadcn_ui` covers MVP component needs cleanly.
- Hot reload accelerates UI iteration significantly.
- `PlatformView` escape hatch lets us replace just the chat surface (one screen) with a UIKit / Android view if benchmarks demand it — without abandoning Flutter for the rest of the app.

**Neutral**
- Flutter text rendering is competitive but not class-leading for sustained ~20 Hz streaming repaints. Mitigation: use an incremental-rendering markdown widget (rebuild only changed deltas, not whole tree) when streaming arrives in P1.
- iOS-native gestures (swipe-back, native sheet presentation) require explicit Cupertino-style reproduction in Flutter. Acceptable trade-off for MVP scope.
- Larger app binary than pure-native.

## Alternatives Rejected

### Twin-native with Telegram UI reuse

Rejected on three independent grounds, any one of which would suffice:

1. **License contamination.** Telegram-iOS is GPL-2.0; Telegram-Android is GPL-2.0/3.0. Linking any GPL UI code makes the entire mobile app GPL. This forecloses commercial paths permanently and conflicts with the Minos MIT licensing intent.
2. **Telegram is a product, not a library.** Streaming markdown rendering is implemented inside Telegram via deeply-coupled internal abstractions (`TextNode`, AsyncDisplayKit, MTProto, Postbox). There is no clean component to "lift". Extraction would be a multi-month research effort followed by a from-scratch reimplementation.
3. **Maintenance cost.** Two UI codebases (Swift + Kotlin), each of which must keep up with platform updates and Telegram-fork drift. At single-developer scale this is not viable.

### Build hooks alone, no flutter_rust_bridge

Dart's `package:hook` (native_assets) is the official Dart-led mechanism for compiling native code as part of `flutter build`. It is mature in Dart 3.5+. However, it provides only the build pipeline — not type-safe Rust ↔ Dart bindings. Without frb you write FFI shims by hand: pointer marshalling, async callback registration, error mapping, type lifetimes, all manual.

Rejected because frb v2 already integrates with native_assets / build hooks and adds the codegen layer on top. There is no version of "use build hooks, skip frb" that saves work.
