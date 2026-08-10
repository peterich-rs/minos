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

## Bot identity SSOT

- **Hub `agents` is bot identity SSOT** (global bot directory + digital body).
- Desktop Agents page: create/list/update/delete prefer Hub (`createCloudAgent` / `listCloudAgents` / `updateCloudAgent` / `deleteCloudAgent`) when the account is online.
- Daemon `agent_profiles` is an **offline / Host launch cache only** — not multi-device identity. Mirror by name is best-effort; never treat local-only profile rows as collab send targets without conversation membership.
- `ensureHostRuntimeAgent` seeds a host_runtime registry slot only — **never** join a conversation.

## `@agent` / `@profile` routing

- **Membership first**: conversation roster (`participatingAgents` / Hub `…/participants` agents) is the SSOT for who may be @mentioned or started. Picker uses `buildAgentMentionOptions({ …, memberAgents })` with **roster-scoped** profiles only. Empty roster ⇒ no options. Non-member start is rejected by daemon / `resolveDispatchTargets`.
- Within the roster, prefer CLI inventory for bare runtime mention rows. `KNOWN_AGENTS` in `shared/lib/agent-route.ts` is an **offline parse fallback only** when clis are empty — not the capability catalog, and not a bypass of membership.
- **Bare `@agent`**: only if that runtime is a member; **reuse** the most recent top-level non-closed session for that runtime when present (desktop). Only when none exists, start a new session and convenience-bind the newest host profile for that runtime (`profile_id` when one exists).
- **`@ProfileName` / `@p/<id>` / Hub bot display name**: bot must be a roster member; always start a **new** session with explicit `profile_id` / agent id when available. Insert `@Name` when the name is unique among roster bots + runtimes; otherwise `@p/<id>`. Runtime names win over same-named profiles at parse time.
- Do **not** offer unjoined Host profiles or bare non-member runtimes as collab @ targets.
- Launch fields are **server-owned**: daemon `resolve_launch_options` loads the profile and applies model / reasoning_effort / instructions. Explicit request fields override profile fields (`explicit > profile > None`). `request.agent` must match `profile.runtime_agent`.
- **MCP** bind the same way and honor membership: clients pass `profile_id` (or MCP `target_profile` name → id); daemon resolves launch options. Bare runtime / `target_agent` convenience-binds the newest host profile when **starting new**. Clients must not merge model/effort/instructions locally.

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
