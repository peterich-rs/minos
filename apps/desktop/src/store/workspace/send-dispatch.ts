/**
 * Desktop user-turn dispatch: participant delivery target resolution only.
 *
 * Aligns with Hub room rules (ADR 0021 / agent-participant-delivery).
 * Bot activation is Hub Agent inbox / Bot mailbox only when Account is live.
 * There is no local fan-out / startAgent collaboration path.
 */

export {
  buildStructuredMentions,
  resolveDispatchTargets,
  type BuildStructuredMentionsOptions,
  type DispatchTarget,
  type ResolveDispatchTargetsInput,
  type WireMentionTarget,
} from "./resolve-dispatch-targets";
