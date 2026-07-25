# Agents feature — capability SSOT

## The one rule

**Harness / runtime capability facts have exactly one source: Rust.**

| Fact | Source |
|------|--------|
| Which runtimes exist | `minos_domain::AgentName` + daemon `list_clis` |
| Display name, model selection, reasoning-effort support | Domain metadata on `AgentName` → fields on `AgentDescriptor` / `CliDto` |
| Per-model effort ladder | Daemon `list_models` → `supported_reasoning_efforts` (honest; empty when unsupported) |

Desktop **projects** these values into UI. It must **not**:

- Hardcode a rival `RUNTIMES` / capability table that invents which agents exist
- Invent a default effort ladder (`["low","medium","high"]`) when the model returns empty efforts
- Use harness id string checks in render to show/hide capability fields (e.g. `if (runtime === "codex") showEffort`)

Presentation-only maps (colors, avatar tone) are fine and may fall back by agent id.

## Projection helpers

`features/agents/lib/agentConfigProjection.ts` is the pure boundary:

- `runtimeOptionsFromClis(clis)` — picker options from store/daemon inventory
- `effortOptionsForModel(model)` — **no fallback ladder**; empty ⇒ hide effort UI
- `shouldShowEffortPicker(model)` / `defaultEffortForModel(model)`

## `@agent` / `@profile` routing

- **Membership first**: conversation roster (`participatingAgents` / `conversation_agent_members`) is the SSOT for who may be @mentioned or started. Picker uses `buildAgentMentionOptions({ …, memberAgents })`. Empty roster ⇒ no options. Non-member start is rejected by daemon.
- Within the roster, prefer CLI inventory for mention rows. `KNOWN_AGENTS` in `shared/lib/agent-route.ts` is an **offline parse fallback only** when clis are empty — not the capability catalog, and not a bypass of membership.
- **Bare `@agent`**: only if that runtime is a member; **reuse** the most recent top-level non-closed session for that runtime when present (desktop + TUI). Only when none exists, start a new session and convenience-bind the newest host profile for that runtime (`profile_id` when one exists).
- **`@ProfileName` / `@p/<id>`**: profile's `runtime_agent` must be a member; always start a **new** session with explicit `profile_id`. Insert `@Name` when the name is unique among profiles + runtimes; otherwise `@p/<id>`. Runtime names win over same-named profiles at parse time.
- Launch fields are **server-owned**: daemon `resolve_launch_options` loads the profile and applies model / reasoning_effort / instructions. Explicit request fields override profile fields (`explicit > profile > None`). `request.agent` must match `profile.runtime_agent`.
- **TUI and MCP** bind the same way and honor membership: clients pass `profile_id` (or MCP `target_profile` name → id); daemon resolves launch options. Bare runtime / `target_agent` convenience-binds the newest host profile when **starting new**. Clients must not merge model/effort/instructions locally.

## How to add a new runtime

1. **Domain first** — add variant to `AgentName` in `crates/minos-domain/src/agent.rs` with `bin_name`, `display_name`, `supports_model_selection`, `supports_reasoning_effort`, `model_discovery`. Extend unit tests over `AgentName::all()`.
2. **Detect** — `minos-cli-detect` probes via `AgentName::all()`; no extra list needed if `bin_name` is correct.
3. **Model catalog** — `minos-daemon/src/model_catalog.rs`: probe and/or static list. Efforts must be honest (empty arrays when the runtime/model does not support effort). Never invent efforts for unsupported runtimes.
4. **Protocol** — `AgentDescriptor` already carries capability fields filled by `AgentDescriptor::new`. Desktop `CliDto` maps them through.
5. **UI projection** — no new hardcoded runtime list. Optional: add presentation entry in `agentMeta` (label/color only).
6. **Do not** regenerate mobile FRB solely for display polish unless mobile needs the new agent; domain + daemon remain SSOT.

## Non-goals (this surface)

- Mobile client host agent profiles (host profiles live on daemon; mobile does not manage them yet)
- Full Agents page visual redesign
- React Query
