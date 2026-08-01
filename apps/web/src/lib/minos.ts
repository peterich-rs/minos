export type DeviceRole = 'browser-admin'
export type AgentName = 'codex' | 'claude' | 'gemini' | 'opencode' | 'grok'
export type MessageRole = 'user' | 'assistant' | 'system'

export type AgentStatus =
  | { kind: 'ok' }
  | { kind: 'missing' }
  | { kind: 'error'; reason: string }

export interface AgentDescriptor {
  name: AgentName
  display_name: string
  path: string | null
  version: string | null
  status: AgentStatus
  supports_model_selection: boolean
  supports_reasoning_effort: boolean
}

export interface AuthSummary {
  account_id: string
  email: string
}

export interface AuthResponse {
  account: AuthSummary
  access_token: string
  refresh_token: string
  expires_in: number
}

export interface StoredSession {
  accountId: string
  email: string
  accessToken: string
  refreshToken: string
}

export interface WsTicketResponse {
  ticket: string
  gateway_url?: string | null
  expires_at_ms?: number | null
}

interface ResponseEnvelope<T> {
  data: T
}

export interface HostSummary {
  host_device_id: string
  host_display_name: string
  paired_at_ms: number
  paired_via_device_id: string
  online?: boolean
}

export interface MeHostsResponse {
  hosts: HostSummary[]
}

interface FormalHostSummary {
  host_installation_id: string
  host_display_name: string | null
  linked_at_ms: number
  online: boolean
}

interface ListHostsData {
  hosts: FormalHostSummary[]
}

export interface MyProfileResponse {
  account_id: string
  email: string
  minos_id: string
  display_name?: string | null
}

export interface UserSummary {
  account_id: string
  minos_id: string
  display_name: string
}

export interface SearchUsersResponse {
  users: UserSummary[]
}

export type FriendRequestStatus =
  | 'pending'
  | 'accepted'
  | 'rejected'
  | 'canceled'

export interface FriendRequestSummary {
  request_id: string
  from: UserSummary
  to: UserSummary
  status: FriendRequestStatus
  created_at_ms: number
  resolved_at_ms?: number | null
}

export interface FriendRequestsResponse {
  incoming: FriendRequestSummary[]
  outgoing: FriendRequestSummary[]
}

export interface FriendSummary {
  account_id: string
  minos_id: string
  display_name: string
  created_at_ms: number
}

export interface FriendsResponse {
  friends: FriendSummary[]
}

export type ConversationKind = 'direct' | 'group'

export interface ConversationSummary {
  conversation_id: string
  kind: ConversationKind
  title: string
  counterpart?: UserSummary | null
  member_count: number
  last_message_preview?: string | null
  last_message_at_ms: number
  unread_count: number
  unread_mention_count: number
}

export interface ConversationsResponse {
  conversations: ConversationSummary[]
}

export interface ConversationResponse {
  conversation_id: string
}

export interface ConversationMembersResponse {
  members: UserSummary[]
}

export interface ConversationReadResponse {
  last_read_at_ms?: number | null
}

export interface ChatMessageReplySummary {
  message_id: string
  sender: UserSummary
  text: string
  recalled_at_ms?: number | null
}

export interface ChatMessageSummary {
  message_id: string
  conversation_id: string
  sender: UserSummary
  text: string
  created_at_ms: number
  reply_to?: ChatMessageReplySummary | null
  recalled_at_ms?: number | null
  mentioned_account_ids?: string[]
}

export interface ListChatMessagesResponse {
  messages: ChatMessageSummary[]
  next_before_ts_ms?: number | null
}

export interface HostSkillSummary {
  name: string
  path: string
  description: string
  enabled: boolean
  scope: string
  display_name?: string | null
  short_description?: string | null
}

export interface HostSkillError {
  path: string
  message: string
}

export interface HostSkillsEntry {
  cwd: string
  errors: HostSkillError[]
  skills: HostSkillSummary[]
}

export interface ListHostSkillsResponse {
  data: HostSkillsEntry[]
}

export interface WriteHostSkillConfigResponse {
  effective_enabled: boolean
}

export interface SessionEndReason {
  kind: 'user_stopped' | 'agent_done' | 'timeout' | 'host_disconnected' | 'crashed'
  message?: string
}

export interface SessionSummary {
  session_id: string
  agent: AgentName
  title: string | null
  first_ts_ms: number
  last_ts_ms: number
  message_count: number
  ended_at_ms: number | null
  end_reason: SessionEndReason | null
}

export interface ListSessionsResponse {
  sessions: SessionSummary[]
  next_before_ts_ms?: number | null
}

export type UiEventMessage =
  | {
      kind: 'session_opened'
      session_id: string
      agent: AgentName
      title: string | null
      opened_at_ms: number
    }
  | {
      kind: 'thread_title_updated'
      session_id: string
      title: string
    }
  | {
      kind: 'thread_closed'
      session_id: string
      reason: SessionEndReason
      closed_at_ms: number
    }
  | {
      kind: 'message_started'
      message_id: string
      role: MessageRole
      started_at_ms: number
    }
  | {
      kind: 'message_completed'
      message_id: string
      finished_at_ms: number
    }
  | {
      kind: 'text_delta'
      message_id: string
      text: string
    }
  | {
      kind: 'text_replace'
      message_id: string
      text: string
    }
  | {
      kind: 'reasoning_delta'
      message_id: string
      text: string
    }
  | {
      kind: 'reasoning_replace'
      message_id: string
      text: string
    }
  | {
      kind: 'tool_call_placed'
      message_id: string
      tool_call_id: string
      name: string
      args_json: string
    }
  | {
      kind: 'tool_call_completed'
      tool_call_id: string
      output: string
      is_error: boolean
    }
  | {
      kind: 'error'
      code: string
      message: string
      message_id: string | null
    }
  | {
      kind: 'raw'
      raw_kind: string
      payload_json: string
    }

export interface ReadSessionResponse {
  ui_events: UiEventMessage[]
  next_seq?: number | null
  session_end_reason?: SessionEndReason | null
}

interface FormalAgentSessionSummary {
  session_id: string
  conversation_id: string
  project_id?: string | null
  agent_id?: string | null
  agent?: AgentName | null
  status: string
  started_at_ms: number
  ended_at_ms?: number | null
  title?: string | null
  last_activity_at_ms: number
  message_count: number
  end_reason?: SessionEndReason | null
}

interface AgentSessionsResponse {
  sessions: FormalAgentSessionSummary[]
  next_before_started_at_ms?: number | null
}

interface AgentTurnMetadata {
  turn_id: string
  turn_seq: number
  role: string
  status: string
  started_at_ms: number
  finished_at_ms?: number | null
  summary_text?: string | null
}

interface AgentTurnEvent {
  turn_id: string
  kind: string
  payload: Record<string, unknown>
  created_at_ms: number
}

interface ReadAgentSessionTurnsResponse {
  turns: AgentTurnMetadata[]
  events: AgentTurnEvent[]
}

export interface StartAgentResponse {
  session_id: string
  cwd: string
}

export interface UiEventFrame {
  session_id: string
  seq: number
  ui: UiEventMessage
  ts_ms: number
}

export interface SocialMessageFrame {
  conversation_id: string
  message: ChatMessageSummary
}

const SESSION_KEY = 'minos.web.session'
const DEVICE_ID_KEY = 'minos.web.device-id'
const ACTIVE_HOST_KEY = 'minos.web.active-host'
const WORKSPACE_KEY = 'minos.web.workspace'
const DEFAULT_BACKEND_URL = 'http://127.0.0.1:8787'

export class MinosHttpError extends Error {
  readonly status: number
  readonly payload?: unknown

  constructor(
    message: string,
    status: number,
    payload?: unknown,
  ) {
    super(message)
    this.status = status
    this.payload = payload
  }
}

export function backendHttpBase(): string {
  const raw = import.meta.env.VITE_MINOS_BACKEND_URL ?? DEFAULT_BACKEND_URL
  return raw.replace(/\/+$/, '')
}

export function backendWsBase(): string {
  return backendHttpBase().replace(/^http/i, 'ws')
}

function storageAvailable(): boolean {
  return typeof window !== 'undefined' && typeof window.localStorage !== 'undefined'
}

export function ensureBrowserDeviceId(): string {
  if (!storageAvailable()) {
    return 'browser-admin-device'
  }
  const current = window.localStorage.getItem(DEVICE_ID_KEY)
  if (current) {
    return current
  }
  const next = window.crypto.randomUUID()
  window.localStorage.setItem(DEVICE_ID_KEY, next)
  return next
}

export function loadStoredSession(): StoredSession | null {
  if (!storageAvailable()) {
    return null
  }
  const raw = window.localStorage.getItem(SESSION_KEY)
  if (!raw) {
    return null
  }
  try {
    return JSON.parse(raw) as StoredSession
  } catch {
    window.localStorage.removeItem(SESSION_KEY)
    return null
  }
}

export function saveStoredSession(session: StoredSession): void {
  if (!storageAvailable()) {
    return
  }
  window.localStorage.setItem(SESSION_KEY, JSON.stringify(session))
}

export function clearStoredSession(): void {
  if (!storageAvailable()) {
    return
  }
  window.localStorage.removeItem(SESSION_KEY)
}

export function loadActiveHost(): string | null {
  if (!storageAvailable()) {
    return null
  }
  return window.localStorage.getItem(ACTIVE_HOST_KEY)
}

export function saveActiveHost(hostId: string | null): void {
  if (!storageAvailable()) {
    return
  }
  if (hostId) {
    window.localStorage.setItem(ACTIVE_HOST_KEY, hostId)
    return
  }
  window.localStorage.removeItem(ACTIVE_HOST_KEY)
}

export function loadWorkspace(): string {
  if (!storageAvailable()) {
    return ''
  }
  return window.localStorage.getItem(WORKSPACE_KEY) ?? ''
}

export function saveWorkspace(workspace: string): void {
  if (!storageAvailable()) {
    return
  }
  window.localStorage.setItem(WORKSPACE_KEY, workspace)
}

function deviceHeaders(deviceId: string, accessToken?: string): HeadersInit {
  const headers: Record<string, string> = {
    'content-type': 'application/json',
    'x-device-id': deviceId,
    'x-device-role': 'browser-admin',
  }
  if (accessToken) {
    headers.authorization = `Bearer ${accessToken}`
  }
  return headers
}

async function parseErrorPayload(response: Response): Promise<unknown> {
  try {
    return await response.json()
  } catch {
    return null
  }
}

function errorMessage(status: number, payload: unknown): string {
  if (payload && typeof payload === 'object') {
    const record = payload as Record<string, unknown>
    const nested = record.error
    if (nested && typeof nested === 'object') {
      const nestedRecord = nested as Record<string, unknown>
      if (typeof nestedRecord.message === 'string') {
        return nestedRecord.message
      }
      if (typeof nestedRecord.code === 'string') {
        return nestedRecord.code
      }
    }
    if (typeof record.kind === 'string') {
      return record.kind
    }
    if (typeof record.message === 'string') {
      return record.message
    }
  }
  return `request failed (${status})`
}

async function requestJson<T>(
  path: string,
  init: RequestInit,
): Promise<T> {
  const response = await fetch(`${backendHttpBase()}${path}`, init)
  if (!response.ok) {
    const payload = await parseErrorPayload(response)
    throw new MinosHttpError(errorMessage(response.status, payload), response.status, payload)
  }
  return response.json() as Promise<T>
}

export async function registerBrowserAccount(
  deviceId: string,
  email: string,
  password: string,
): Promise<AuthResponse> {
  return requestJson<AuthResponse>('/v1/auth/register', {
    method: 'POST',
    headers: deviceHeaders(deviceId),
    body: JSON.stringify({ email, password }),
  })
}

export async function loginBrowserAccount(
  deviceId: string,
  email: string,
  password: string,
): Promise<AuthResponse> {
  return requestJson<AuthResponse>('/v1/auth/login', {
    method: 'POST',
    headers: deviceHeaders(deviceId),
    body: JSON.stringify({ email, password }),
  })
}

/** Exchange a Supabase access token for Minos access/refresh tokens. */
export async function exchangeSupabaseSession(
  deviceId: string,
  supabaseAccessToken: string,
  deviceName?: string,
): Promise<AuthResponse> {
  return requestJson<AuthResponse>('/v1/auth/supabase', {
    method: 'POST',
    headers: deviceHeaders(deviceId),
    body: JSON.stringify({
      access_token: supabaseAccessToken,
      ...(deviceName ? { device_name: deviceName } : {}),
    }),
  })
}

export async function refreshBrowserSession(
  deviceId: string,
  refreshToken: string,
): Promise<AuthResponse> {
  const response = await requestJson<{
    access_token: string
    refresh_token: string
    expires_in: number
  }>('/v1/auth/refresh', {
    method: 'POST',
    headers: deviceHeaders(deviceId),
    body: JSON.stringify({ refresh_token: refreshToken }),
  })

  return {
    account: {
      account_id: '',
      email: '',
    },
    access_token: response.access_token,
    refresh_token: response.refresh_token,
    expires_in: response.expires_in,
  }
}

export async function createWsTicket(
  deviceId: string,
  accessToken: string,
): Promise<WsTicketResponse> {
  const envelope = await requestJson<ResponseEnvelope<{
    ticket: string
    gateway_url: string
    expires_at_ms: number
  }>>('/v1/realtime/ws-ticket', {
    method: 'POST',
    headers: deviceHeaders(deviceId, accessToken),
    body: JSON.stringify({ installation_id: deviceId }),
  })
  return {
    ticket: envelope.data.ticket,
    gateway_url: envelope.data.gateway_url,
    expires_at_ms: envelope.data.expires_at_ms,
  }
}

function requestAuthedQuery<T>(
  path: string,
  deviceId: string,
  accessToken: string,
  body: Record<string, unknown> = {},
): Promise<T> {
  return requestJson<T>(path, {
    method: 'POST',
    headers: deviceHeaders(deviceId, accessToken),
    body: JSON.stringify(body),
  })
}

/** List Host-Link hosts for the account (`GET /v1/hosts`). */
export async function listHosts(
  deviceId: string,
  accessToken: string,
): Promise<MeHostsResponse> {
  const envelope = await requestJson<ResponseEnvelope<ListHostsData>>('/v1/hosts', {
    method: 'GET',
    headers: deviceHeaders(deviceId, accessToken),
  })
  return {
    hosts: envelope.data.hosts.map((host) => ({
      host_device_id: host.host_installation_id,
      host_display_name: host.host_display_name ?? 'Mac',
      paired_at_ms: host.linked_at_ms,
      paired_via_device_id: '',
      online: host.online,
    })),
  }
}

export async function getMyProfile(
  deviceId: string,
  accessToken: string,
): Promise<MyProfileResponse> {
  return requestAuthedQuery<MyProfileResponse>(
    '/v1/profiles/self',
    deviceId,
    accessToken,
  )
}

export async function setMyMinosId(
  deviceId: string,
  accessToken: string,
  minosId: string,
): Promise<MyProfileResponse> {
  return requestJson<MyProfileResponse>('/v1/profiles/minos-id', {
    method: 'POST',
    headers: deviceHeaders(deviceId, accessToken),
    body: JSON.stringify({ minos_id: minosId }),
  })
}

export async function setMyDisplayName(
  deviceId: string,
  accessToken: string,
  displayName: string | null,
): Promise<MyProfileResponse> {
  return requestJson<MyProfileResponse>('/v1/profiles/display-name', {
    method: 'POST',
    headers: deviceHeaders(deviceId, accessToken),
    body: JSON.stringify({ display_name: displayName ?? null }),
  })
}

export async function changePassword(
  deviceId: string,
  accessToken: string,
  currentPassword: string,
  newPassword: string,
): Promise<void> {
  const response = await fetch(`${backendHttpBase()}/v1/auth/change-password`, {
    method: 'POST',
    headers: deviceHeaders(deviceId, accessToken),
    body: JSON.stringify({
      current_password: currentPassword,
      new_password: newPassword,
    }),
  })
  if (!response.ok) {
    const payload = await parseErrorPayload(response)
    throw new MinosHttpError(errorMessage(response.status, payload), response.status, payload)
  }
}

export async function searchUsers(
  deviceId: string,
  accessToken: string,
  minosId: string,
): Promise<SearchUsersResponse> {
  return requestAuthedQuery<SearchUsersResponse>(
    '/v1/profiles/search',
    deviceId,
    accessToken,
    { minos_id: minosId },
  )
}

export async function listFriendRequests(
  deviceId: string,
  accessToken: string,
): Promise<FriendRequestsResponse> {
  return requestAuthedQuery<FriendRequestsResponse>(
    '/v1/friend-requests/query',
    deviceId,
    accessToken,
  )
}

export async function createFriendRequest(
  deviceId: string,
  accessToken: string,
  targetMinosId: string,
): Promise<FriendRequestSummary> {
  return requestJson<FriendRequestSummary>('/v1/friend-requests', {
    method: 'POST',
    headers: deviceHeaders(deviceId, accessToken),
    body: JSON.stringify({ target_minos_id: targetMinosId }),
  })
}

export async function acceptFriendRequest(
  deviceId: string,
  accessToken: string,
  requestId: string,
): Promise<FriendRequestSummary> {
  return requestJson<FriendRequestSummary>(
    `/v1/friend-requests/${requestId}/accept`,
    {
      method: 'POST',
      headers: deviceHeaders(deviceId, accessToken),
      body: JSON.stringify({}),
    },
  )
}

export async function rejectFriendRequest(
  deviceId: string,
  accessToken: string,
  requestId: string,
): Promise<FriendRequestSummary> {
  return requestJson<FriendRequestSummary>(
    `/v1/friend-requests/${requestId}/reject`,
    {
      method: 'POST',
      headers: deviceHeaders(deviceId, accessToken),
      body: JSON.stringify({}),
    },
  )
}

export async function listFriends(
  deviceId: string,
  accessToken: string,
): Promise<FriendsResponse> {
  return requestAuthedQuery<FriendsResponse>(
    '/v1/friends/query',
    deviceId,
    accessToken,
  )
}

export async function listConversations(
  deviceId: string,
  accessToken: string,
): Promise<ConversationsResponse> {
  return requestAuthedQuery<ConversationsResponse>(
    '/v1/conversations/query',
    deviceId,
    accessToken,
  )
}

export async function ensureDirectConversation(
  deviceId: string,
  accessToken: string,
  friendAccountId: string,
): Promise<ConversationResponse> {
  return requestJson<ConversationResponse>('/v1/conversations/direct', {
    method: 'POST',
    headers: deviceHeaders(deviceId, accessToken),
    body: JSON.stringify({ friend_account_id: friendAccountId }),
  })
}

export async function createGroupConversation(
  deviceId: string,
  accessToken: string,
  title: string,
  memberAccountIds: string[],
): Promise<ConversationResponse> {
  return requestJson<ConversationResponse>('/v1/conversations/group', {
    method: 'POST',
    headers: deviceHeaders(deviceId, accessToken),
    body: JSON.stringify({
      title,
      member_account_ids: memberAccountIds,
    }),
  })
}

export async function listConversationMembers(
  deviceId: string,
  accessToken: string,
  conversationId: string,
): Promise<ConversationMembersResponse> {
  return requestAuthedQuery<ConversationMembersResponse>(
    `/v1/conversations/${conversationId}/members/query`,
    deviceId,
    accessToken,
  )
}

export async function markConversationRead(
  deviceId: string,
  accessToken: string,
  conversationId: string,
): Promise<ConversationReadResponse> {
  return requestJson<ConversationReadResponse>(
    `/v1/conversations/${conversationId}/read`,
    {
      method: 'POST',
      headers: deviceHeaders(deviceId, accessToken),
      body: JSON.stringify({}),
    },
  )
}

export async function listConversationMessages(
  deviceId: string,
  accessToken: string,
  conversationId: string,
  options: { beforeTsMs?: number | null; limit?: number } = {},
): Promise<ListChatMessagesResponse> {
  return requestAuthedQuery<ListChatMessagesResponse>(
    `/v1/conversations/${conversationId}/messages/query`,
    deviceId,
    accessToken,
    {
      limit: options.limit ?? 50,
      before_ts_ms: options.beforeTsMs ?? undefined,
    },
  )
}

export async function sendConversationMessage(
  deviceId: string,
  accessToken: string,
  conversationId: string,
  text: string,
  replyToMessageId?: string | null,
): Promise<ChatMessageSummary> {
  return requestJson<ChatMessageSummary>(
    `/v1/conversations/${conversationId}/messages`,
    {
      method: 'POST',
      headers: deviceHeaders(deviceId, accessToken),
      body: JSON.stringify({
        text,
        reply_to_message_id: replyToMessageId ?? null,
      }),
    },
  )
}

export async function recallConversationMessage(
  deviceId: string,
  accessToken: string,
  conversationId: string,
  messageId: string,
): Promise<ChatMessageSummary> {
  return requestJson<ChatMessageSummary>(
    `/v1/conversations/${conversationId}/messages/${messageId}/recall`,
    {
      method: 'POST',
      headers: deviceHeaders(deviceId, accessToken),
      body: JSON.stringify({}),
    },
  )
}

export async function listThreads(
  deviceId: string,
  accessToken: string,
  options: { limit?: number; beforeTsMs?: number | null; agent?: AgentName | null } = {},
): Promise<ListSessionsResponse> {
  const body = await requestAuthedQuery<AgentSessionsResponse>(
    '/v1/agent-sessions/list',
    deviceId,
    accessToken,
    {
      limit: options.limit ?? 50,
      before_started_at_ms: options.beforeTsMs ?? undefined,
    },
  )
  const sessions = body.sessions.map(threadSummaryFromFormalSession)
  return {
    sessions: options.agent ? sessions.filter((thread) => thread.agent === options.agent) : sessions,
    next_before_ts_ms: body.next_before_started_at_ms,
  }
}

export async function readThread(
  deviceId: string,
  accessToken: string,
  sessionId: string,
): Promise<ReadSessionResponse> {
  const turnsPage = await requestAuthedQuery<ReadAgentSessionTurnsResponse>(
    '/v1/agent-sessions/read-turns',
    deviceId,
    accessToken,
    {
      session_id: sessionId,
      limit: 200,
    },
  )
  const uiEvents: UiEventMessage[] = []
  for (const turn of turnsPage.turns) {
    const eventPage = await requestAuthedQuery<ReadAgentSessionTurnsResponse>(
      '/v1/agent-sessions/read-turns',
      deviceId,
      accessToken,
      {
        turn_id: turn.turn_id,
        limit: 200,
      },
    )
    appendTurnUiEvents(uiEvents, turn, eventPage.events)
  }
  return {
    ui_events: uiEvents,
    next_seq: turnsPage.turns.length >= 200
      ? turnsPage.turns[turnsPage.turns.length - 1]?.turn_seq
      : null,
    session_end_reason: null,
  }
}

function threadSummaryFromFormalSession(session: FormalAgentSessionSummary): SessionSummary {
  return {
    session_id: session.session_id,
    agent: session.agent ?? agentNameFromSessionAgentId(session.agent_id) ?? 'codex',
    title: session.title ?? null,
    first_ts_ms: session.started_at_ms,
    last_ts_ms: session.last_activity_at_ms,
    message_count: session.message_count,
    ended_at_ms: session.ended_at_ms ?? null,
    end_reason: session.end_reason ?? null,
  }
}

function agentNameFromSessionAgentId(agentId?: string | null): AgentName | null {
  if (!agentId) return null
  if (agentId === 'codex' || agentId.startsWith('codex-')) return 'codex'
  if (agentId === 'claude' || agentId.startsWith('claude-')) return 'claude'
  if (agentId === 'gemini' || agentId.startsWith('gemini-')) return 'gemini'
  return null
}

function appendTurnUiEvents(
  uiEvents: UiEventMessage[],
  turn: AgentTurnMetadata,
  events: AgentTurnEvent[],
): void {
  if (events.length === 0) {
    appendTurnSummaryUiEvents(uiEvents, turn)
    return
  }

  const role = messageRoleFromTurnRole(turn.role)
  const startedMessageIds = new Set<string>()
  for (const event of events) {
    const messageId = messageIdForTurnEvent(turn, event)
    if (!startedMessageIds.has(messageId)) {
      startedMessageIds.add(messageId)
      uiEvents.push({
        kind: 'message_started',
        message_id: messageId,
        role,
        started_at_ms: turn.started_at_ms,
      })
    }
    uiEvents.push(...uiEventsForTurnEvent(messageId, event))
  }

  if (turnIsComplete(turn)) {
    const finishedAtMs = turn.finished_at_ms ?? turn.started_at_ms
    for (const messageId of startedMessageIds) {
      uiEvents.push({
        kind: 'message_completed',
        message_id: messageId,
        finished_at_ms: finishedAtMs,
      })
    }
  }
}

function appendTurnSummaryUiEvents(uiEvents: UiEventMessage[], turn: AgentTurnMetadata): void {
  if (!turn.summary_text) return
  uiEvents.push({
    kind: 'message_started',
    message_id: turn.turn_id,
    role: messageRoleFromTurnRole(turn.role),
    started_at_ms: turn.started_at_ms,
  })
  uiEvents.push({
    kind: 'text_delta',
    message_id: turn.turn_id,
    text: turn.summary_text,
  })
  if (turnIsComplete(turn)) {
    uiEvents.push({
      kind: 'message_completed',
      message_id: turn.turn_id,
      finished_at_ms: turn.finished_at_ms ?? turn.started_at_ms,
    })
  }
}

function uiEventsForTurnEvent(messageId: string, event: AgentTurnEvent): UiEventMessage[] {
  switch (event.kind) {
    case 'agent_text_delta':
      return textFromPayload(event.payload).map((text) => ({
        kind: 'text_delta' as const,
        message_id: messageId,
        text,
      }))
    case 'agent_text_replace':
      return textFromPayload(event.payload).map((text) => ({
        kind: 'text_replace' as const,
        message_id: messageId,
        text,
      }))
    case 'agent_reasoning_delta':
      return textFromPayload(event.payload).map((text) => ({
        kind: 'reasoning_delta' as const,
        message_id: messageId,
        text,
      }))
    case 'agent_reasoning_replace':
      return textFromPayload(event.payload).map((text) => ({
        kind: 'reasoning_replace' as const,
        message_id: messageId,
        text,
      }))
    case 'agent_tool_call':
      return [{
        kind: 'tool_call_placed',
        message_id: messageId,
        tool_call_id: stringPayload(event.payload, ['tool_call_id', 'id']) ?? 'tool_call',
        name: stringPayload(event.payload, ['name', 'tool_name']) ?? 'tool',
        args_json: argsJsonFromPayload(event.payload),
      }]
    case 'agent_tool_result':
    case 'agent_tool_completed':
      return [{
        kind: 'tool_call_completed',
        tool_call_id: stringPayload(event.payload, ['tool_call_id', 'id']) ?? 'tool_call',
        output: outputFromPayload(event.payload),
        is_error: Boolean(event.payload.is_error ?? event.payload.error ?? false),
      }]
    case 'agent_error':
      return [{
        kind: 'error',
        code: stringPayload(event.payload, ['code']) ?? 'agent_error',
        message: stringPayload(event.payload, ['message', 'detail']) ?? 'agent error',
        message_id: messageId,
      }]
    default:
      return [{
        kind: 'raw',
        raw_kind: event.kind,
        payload_json: JSON.stringify(event.payload),
      }]
  }
}

function messageIdForTurnEvent(turn: AgentTurnMetadata, event: AgentTurnEvent): string {
  return stringPayload(event.payload, ['message_id', 'msg_id']) || event.turn_id || turn.turn_id
}

function textFromPayload(payload: Record<string, unknown>): string[] {
  const value = stringPayload(payload, ['text', 'delta', 'content'])
  return value ? [value] : []
}

function stringPayload(payload: Record<string, unknown>, keys: string[]): string | null {
  for (const key of keys) {
    const value = payload[key]
    if (typeof value === 'string') return value
  }
  return null
}

function argsJsonFromPayload(payload: Record<string, unknown>): string {
  const argsJson = stringPayload(payload, ['args_json'])
  if (argsJson) return argsJson
  const args = payload.args
  return args === undefined ? '{}' : JSON.stringify(args)
}

function outputFromPayload(payload: Record<string, unknown>): string {
  const value = payload.output ?? payload.result
  if (value === undefined || value === null) return ''
  return typeof value === 'string' ? value : JSON.stringify(value)
}

function messageRoleFromTurnRole(role: string): MessageRole {
  return role === 'user' || role === 'system' ? role : 'assistant'
}

function turnIsComplete(turn: AgentTurnMetadata): boolean {
  return ['completed', 'failed', 'cancelled', 'canceled'].includes(turn.status)
    || turn.finished_at_ms != null
}

export async function logoutBrowserSession(
  deviceId: string,
  accessToken: string,
  refreshToken: string,
): Promise<void> {
  await fetch(`${backendHttpBase()}/v1/auth/logout`, {
    method: 'POST',
    headers: deviceHeaders(deviceId, accessToken),
    body: JSON.stringify({ refresh_token: refreshToken }),
  })
}

export async function runWithSessionRefresh<T>(
  session: StoredSession,
  deviceId: string,
  commitSession: (nextSession: StoredSession) => void,
  operation: (current: StoredSession) => Promise<T>,
  allowRefresh = true,
): Promise<T> {
  try {
    return await operation(session)
  } catch (error) {
    if (
      allowRefresh &&
      error instanceof MinosHttpError &&
      error.status === 401
    ) {
      const refreshed = await refreshBrowserSession(
        deviceId,
        session.refreshToken,
      )
      const nextSession: StoredSession = {
        accountId: session.accountId,
        email: session.email,
        accessToken: refreshed.access_token,
        refreshToken: refreshed.refresh_token,
      }
      saveStoredSession(nextSession)
      commitSession(nextSession)
      return operation(nextSession)
    }
    throw error
  }
}

// Re-export RelaySocket from its dedicated module for backwards compatibility
export { RelaySocket } from './relay-socket'
export type { RelayConnectionState, RelaySocketOptions, AutoReconnectOptions } from './relay-socket'
