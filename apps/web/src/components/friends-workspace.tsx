import { useEffect, useRef, useState } from 'react'
import { motion, AnimatePresence } from 'framer-motion'
import {
  AtSign,
  Check,
  MailPlus,
  MessageSquareMore,
  RefreshCw,
  Send,
  Undo2,
  UserPlus,
  Users,
  X,
} from 'lucide-react'
import { toast } from 'sonner'

import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Textarea } from '@/components/ui/textarea'
import {
  type ChatMessageSummary,
  type ConversationSummary,
  type FriendRequestsResponse,
  type FriendSummary,
  type MyProfileResponse,
  type SearchUsersResponse,
  type SocialMessageFrame,
  type StoredSession,
  type UserSummary,
  acceptFriendRequest,
  createFriendRequest,
  createGroupConversation,
  ensureDirectConversation,
  getMyProfile,
  listConversationMembers,
  listConversationMessages,
  listConversations,
  listFriendRequests,
  listFriends,
  markConversationRead,
  recallConversationMessage,
  rejectFriendRequest,
  runWithSessionRefresh,
  searchUsers,
  sendConversationMessage,
  senderHandle,
  senderIsMine,
} from '@/lib/minos'
import { useAppStore } from '@/lib/store'
import { cn } from '@/lib/utils'

function formatClock(ms: number): string {
  return new Intl.DateTimeFormat(undefined, { hour: '2-digit', minute: '2-digit' }).format(ms)
}

function formatRelative(ms: number): string {
  const delta = Math.round((Date.now() - ms) / 60_000)
  if (delta <= 1) return '刚刚'
  if (delta < 60) return `${delta} 分钟前`
  const h = Math.round(delta / 60)
  if (h < 24) return `${h} 小时前`
  const d = Math.round(h / 24)
  return `${d} 天前`
}

function sortConversations(items: ConversationSummary[]): ConversationSummary[] {
  return [...items].sort((a, b) => b.last_message_at_ms - a.last_message_at_ms)
}

function previewForMessage(m: ChatMessageSummary): string {
  if (m.recalled_at_ms) return '消息已撤回'
  const trimmed = m.text.trim()
  if (!trimmed) return '新消息'
  return trimmed.length > 88 ? `${trimmed.slice(0, 88)}…` : trimmed
}

function upsertMessages(
  messages: ChatMessageSummary[],
  next: ChatMessageSummary,
): ChatMessageSummary[] {
  const existing = messages.findIndex((m) => m.message_id === next.message_id)
  if (existing === -1) {
    return [...messages, next].sort((a, b) => a.created_at_ms - b.created_at_ms)
  }
  return messages.map((m, i) => (i === existing ? next : m))
}

async function loadDirectorySnapshot(
  session: StoredSession,
  deviceId: string,
  commitSession: (next: StoredSession) => void,
) {
  const [profile, requests, friends, conversations] = await runWithSessionRefresh(
    session,
    deviceId,
    commitSession,
    (current) =>
      Promise.all([
        getMyProfile(deviceId, current.accessToken),
        listFriendRequests(deviceId, current.accessToken),
        listFriends(deviceId, current.accessToken),
        listConversations(deviceId, current.accessToken),
      ]),
  )
  return {
    profile,
    requests,
    friends: friends.friends,
    conversations: sortConversations(conversations.conversations),
  }
}

export function FriendsWorkspace() {
  const { deviceId, session, setSession, latestSocialEvent } = useAppStore()
  const activeSession = session!

  const [profile, setProfile] = useState<MyProfileResponse | null>(null)
  const [requests, setRequests] = useState<FriendRequestsResponse>({ incoming: [], outgoing: [] })
  const [friends, setFriends] = useState<FriendSummary[]>([])
  const [conversations, setConversations] = useState<ConversationSummary[]>([])
  const [selectedConversationId, setSelectedConversationId] = useState<string | null>(null)
  const [conversationMembers, setConversationMembers] = useState<Record<string, UserSummary[]>>({})
  const [conversationMessages, setConversationMessages] = useState<Record<string, ChatMessageSummary[]>>({})
  const [bootBusy, setBootBusy] = useState(true)
  const [searchQuery, setSearchQuery] = useState('')
  const [searchBusy, setSearchBusy] = useState(false)
  const [searchResults, setSearchResults] = useState<SearchUsersResponse['users']>([])
  const [groupDialogOpen, setGroupDialogOpen] = useState(false)
  const [groupTitle, setGroupTitle] = useState('')
  const [selectedGroupMembers, setSelectedGroupMembers] = useState<string[]>([])
  const [groupBusy, setGroupBusy] = useState(false)
  const [composer, setComposer] = useState('')
  const [sendBusy, setSendBusy] = useState(false)
  const [replyTarget, setReplyTarget] = useState<ChatMessageSummary | null>(null)
  const [mentionOpen, setMentionOpen] = useState(false)
  const [recallingId, setRecallingId] = useState<string | null>(null)
  const lastEventKey = useRef<string | null>(null)
  const messagesRef = useRef(conversationMessages)
  const messageScrollRef = useRef<HTMLDivElement>(null)

  const activeConversation =
    conversations.find((c) => c.conversation_id === selectedConversationId) ?? null
  const activeMembers = selectedConversationId ? conversationMembers[selectedConversationId] ?? [] : []
  const activeMessages = selectedConversationId ? conversationMessages[selectedConversationId] ?? [] : []
  const pendingIncoming = requests.incoming.filter((r) => r.status === 'pending')
  const pendingOutgoing = requests.outgoing.filter((r) => r.status === 'pending')

  useEffect(() => {
    messagesRef.current = conversationMessages
  }, [conversationMessages])

  async function refreshDirectory() {
    try {
      const snapshot = await loadDirectorySnapshot(activeSession, deviceId, setSession)
      setProfile(snapshot.profile)
      setRequests(snapshot.requests)
      setFriends(snapshot.friends)
      setConversations(snapshot.conversations)
      setSelectedConversationId((current) => {
        if (current && snapshot.conversations.some((c) => c.conversation_id === current)) {
          return current
        }
        return snapshot.conversations[0]?.conversation_id ?? null
      })
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e))
    }
  }

  useEffect(() => {
    let cancelled = false
    ;(async () => {
      try {
        const snapshot = await loadDirectorySnapshot(activeSession, deviceId, setSession)
        if (cancelled) return
        setProfile(snapshot.profile)
        setRequests(snapshot.requests)
        setFriends(snapshot.friends)
        setConversations(snapshot.conversations)
        setSelectedConversationId(snapshot.conversations[0]?.conversation_id ?? null)
      } catch (e) {
        if (!cancelled) toast.error(e instanceof Error ? e.message : String(e))
      } finally {
        if (!cancelled) setBootBusy(false)
      }
    })()
    return () => {
      cancelled = true
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  useEffect(() => {
    const trimmed = searchQuery.trim()
    if (!trimmed) {
      // Reset local search state when the query is cleared.
      // eslint-disable-next-line react-hooks/set-state-in-effect
      if (searchResults.length > 0) setSearchResults([])
      if (searchBusy) setSearchBusy(false)
      return
    }
    let cancelled = false
    const timer = window.setTimeout(() => {
      setSearchBusy(true)
      runWithSessionRefresh(activeSession, deviceId, setSession, (current) =>
        searchUsers(deviceId, current.accessToken, trimmed),
      )
        .then((response) => {
          if (!cancelled) setSearchResults(response.users)
        })
        .catch((e) => {
          if (!cancelled) toast.error(e instanceof Error ? e.message : String(e))
        })
        .finally(() => {
          if (!cancelled) setSearchBusy(false)
        })
    }, 260)
    return () => {
      cancelled = true
      window.clearTimeout(timer)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [searchQuery])

  useEffect(() => {
    if (!selectedConversationId) return
    let cancelled = false
    ;(async () => {
      try {
        const [members, messages] = await runWithSessionRefresh(
          activeSession,
          deviceId,
          setSession,
          async (current) => {
            const result = await Promise.all([
              listConversationMembers(deviceId, current.accessToken, selectedConversationId),
              listConversationMessages(deviceId, current.accessToken, selectedConversationId),
              markConversationRead(deviceId, current.accessToken, selectedConversationId),
            ])
            return result
          },
        )
        if (cancelled) return
        setConversationMembers((prev) => ({
          ...prev,
          [selectedConversationId]: members.members,
        }))
        setConversationMessages((prev) => ({
          ...prev,
          [selectedConversationId]: messages.messages,
        }))
        setConversations((prev) =>
          prev.map((c) =>
            c.conversation_id === selectedConversationId
              ? { ...c, unread_count: 0, unread_mention_count: 0 }
              : c,
          ),
        )
      } catch (e) {
        if (!cancelled) toast.error(e instanceof Error ? e.message : String(e))
      }
    })()
    return () => {
      cancelled = true
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedConversationId])

  useEffect(() => {
    if (!latestSocialEvent) return
    const event: SocialMessageFrame = latestSocialEvent
    const key = [
      event.conversation_id,
      event.message.message_id,
      event.message.recalled_at_ms ?? event.message.created_at_ms,
    ].join(':')
    if (lastEventKey.current === key) return
    lastEventKey.current = key

    const isActive = event.conversation_id === selectedConversationId
    const alreadyLoaded = Boolean(
      messagesRef.current[event.conversation_id]?.some(
        (m) => m.message_id === event.message.message_id,
      ),
    )
    setConversationMessages((prev) => ({
      ...prev,
      [event.conversation_id]: upsertMessages(prev[event.conversation_id] ?? [], event.message),
    }))
    setConversations((prev) =>
      sortConversations(
        prev.map((c) => {
          if (c.conversation_id !== event.conversation_id) return c
          const fromMe = senderIsMine(event.message.sender, activeSession.accountId)
          const isRecall = Boolean(event.message.recalled_at_ms)
          const inc =
            !isActive && !alreadyLoaded && !isRecall && !fromMe
          // Monotonic list clock: recall/stale frames must not regress last activity.
          const incomingAt =
            typeof event.message.created_at_ms === 'number' &&
            event.message.created_at_ms > 0
              ? event.message.created_at_ms
              : 0
          const lastAt =
            incomingAt > 0
              ? Math.max(c.last_message_at_ms ?? 0, incomingAt)
              : (c.last_message_at_ms ?? 0)
          return {
            ...c,
            last_message_at_ms: lastAt,
            last_message_preview: previewForMessage(event.message),
            unread_count: inc ? c.unread_count + 1 : isActive ? 0 : c.unread_count,
            unread_mention_count:
              inc && (event.message.mentioned_account_ids ?? []).includes(activeSession.accountId)
                ? c.unread_mention_count + 1
                : isActive
                ? 0
                : c.unread_mention_count,
          }
        }),
      ),
    )
    if (isActive && !senderIsMine(event.message.sender, activeSession.accountId)) {
      runWithSessionRefresh(activeSession, deviceId, setSession, (current) =>
        markConversationRead(deviceId, current.accessToken, event.conversation_id),
      ).catch(() => {})
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [latestSocialEvent])

  useEffect(() => {
    if (messageScrollRef.current) {
      messageScrollRef.current.scrollTop = messageScrollRef.current.scrollHeight
    }
  }, [activeMessages.length, selectedConversationId])

  async function handleAddFriend(targetMinosId: string) {
    try {
      await runWithSessionRefresh(activeSession, deviceId, setSession, (current) =>
        createFriendRequest(deviceId, current.accessToken, targetMinosId),
      )
      const nextRequests = await runWithSessionRefresh(activeSession, deviceId, setSession, (current) =>
        listFriendRequests(deviceId, current.accessToken),
      )
      setRequests(nextRequests)
      toast.success(`已向 @${targetMinosId} 发送好友请求`)
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e))
    }
  }

  async function handleResolve(requestId: string, action: 'accept' | 'reject') {
    try {
      if (action === 'accept') {
        await runWithSessionRefresh(activeSession, deviceId, setSession, (current) =>
          acceptFriendRequest(deviceId, current.accessToken, requestId),
        )
        toast.success('已接受好友请求')
      } else {
        await runWithSessionRefresh(activeSession, deviceId, setSession, (current) =>
          rejectFriendRequest(deviceId, current.accessToken, requestId),
        )
        toast('已拒绝好友请求')
      }
      await refreshDirectory()
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e))
    }
  }

  async function openDirect(friend: FriendSummary) {
    try {
      const response = await runWithSessionRefresh(activeSession, deviceId, setSession, (current) =>
        ensureDirectConversation(deviceId, current.accessToken, friend.account_id),
      )
      const list = await runWithSessionRefresh(activeSession, deviceId, setSession, (current) =>
        listConversations(deviceId, current.accessToken),
      )
      setConversations(sortConversations(list.conversations))
      setSelectedConversationId(response.conversation_id)
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e))
    }
  }

  async function handleCreateGroup() {
    if (selectedGroupMembers.length < 2) {
      toast.error('群聊至少需要 2 位好友')
      return
    }
    try {
      setGroupBusy(true)
      const response = await runWithSessionRefresh(activeSession, deviceId, setSession, (current) =>
        createGroupConversation(
          deviceId,
          current.accessToken,
          groupTitle.trim() || '新群聊',
          selectedGroupMembers,
        ),
      )
      const list = await runWithSessionRefresh(activeSession, deviceId, setSession, (current) =>
        listConversations(deviceId, current.accessToken),
      )
      setConversations(sortConversations(list.conversations))
      setSelectedConversationId(response.conversation_id)
      setGroupDialogOpen(false)
      setGroupTitle('')
      setSelectedGroupMembers([])
      toast.success('群聊已创建')
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e))
    } finally {
      setGroupBusy(false)
    }
  }

  async function handleSend() {
    if (!selectedConversationId || !composer.trim()) return
    try {
      setSendBusy(true)
      const next = await runWithSessionRefresh(activeSession, deviceId, setSession, (current) =>
        sendConversationMessage(
          deviceId,
          current.accessToken,
          selectedConversationId,
          composer.trim(),
          replyTarget?.message_id,
        ),
      )
      setConversationMessages((prev) => ({
        ...prev,
        [selectedConversationId]: upsertMessages(prev[selectedConversationId] ?? [], next),
      }))
      setConversations((prev) =>
        sortConversations(
          prev.map((c) =>
            c.conversation_id === selectedConversationId
              ? {
                  ...c,
                  last_message_at_ms: Math.max(
                    c.last_message_at_ms ?? 0,
                    next.created_at_ms > 0 ? next.created_at_ms : 0,
                  ),
                  last_message_preview: previewForMessage(next),
                }
              : c,
          ),
        ),
      )
      setComposer('')
      setReplyTarget(null)
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e))
    } finally {
      setSendBusy(false)
    }
  }

  async function handleRecall(message: ChatMessageSummary) {
    if (!selectedConversationId) return
    try {
      setRecallingId(message.message_id)
      const recalled = await runWithSessionRefresh(activeSession, deviceId, setSession, (current) =>
        recallConversationMessage(
          deviceId,
          current.accessToken,
          selectedConversationId,
          message.message_id,
        ),
      )
      setConversationMessages((prev) => ({
        ...prev,
        [selectedConversationId]: upsertMessages(prev[selectedConversationId] ?? [], recalled),
      }))
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e))
    } finally {
      setRecallingId(null)
    }
  }

  function toggleGroupMember(accountId: string) {
    setSelectedGroupMembers((prev) =>
      prev.includes(accountId) ? prev.filter((a) => a !== accountId) : [...prev, accountId],
    )
  }

  function insertMention(member: UserSummary) {
    setComposer(
      (prev) => `${prev}${prev.endsWith(' ') || prev.length === 0 ? '' : ' '}@${member.minos_id} `,
    )
    setMentionOpen(false)
  }

  return (
    <div className="flex h-full gap-4 p-4">
      {/* Left — friends + requests */}
      <aside className="flex w-80 shrink-0 flex-col gap-4">
        <section className="flex flex-col rounded-2xl border border-border/70 bg-card p-4 shadow-sm">
          <div className="flex items-center justify-between">
            <div>
              <p className="mono text-[10px] uppercase tracking-[0.18em] text-muted-foreground">
                Profile
              </p>
              <h3 className="text-sm font-semibold">
                {profile?.display_name?.trim() || profile?.minos_id || activeSession.email}
              </h3>
            </div>
            <Badge variant="outline" className="mono">
              @{profile?.minos_id ?? '...'}
            </Badge>
          </div>
        </section>

        <section className="flex flex-col gap-3 rounded-2xl border border-border/70 bg-card p-4 shadow-sm">
          <div>
            <p className="mono text-[10px] uppercase tracking-[0.18em] text-muted-foreground">
              Add friends
            </p>
            <h3 className="text-sm font-semibold">通过 Minos ID 搜索</h3>
          </div>
          <Input
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            placeholder="alice01"
          />
          <div className="max-h-64 overflow-auto">
            {searchBusy ? (
              <p className="py-4 text-center text-xs text-muted-foreground">搜索中…</p>
            ) : !searchQuery.trim() ? (
              <p className="py-4 text-center text-xs text-muted-foreground">
                开始输入 Minos ID 以发现用户
              </p>
            ) : searchResults.length === 0 ? (
              <p className="py-4 text-center text-xs text-muted-foreground">未找到匹配用户</p>
            ) : (
              <div className="flex flex-col gap-1">
                {searchResults.map((user) => (
                  <div
                    key={user.account_id}
                    className="flex items-center justify-between rounded-lg border border-border/60 bg-background/40 px-3 py-2"
                  >
                    <div className="min-w-0">
                      <div className="truncate text-sm font-medium">{user.display_name}</div>
                      <div className="mono text-[11px] text-muted-foreground">@{user.minos_id}</div>
                    </div>
                    <Button
                      size="sm"
                      variant="secondary"
                      onClick={() => handleAddFriend(user.minos_id)}
                    >
                      <MailPlus size={12} />
                      添加
                    </Button>
                  </div>
                ))}
              </div>
            )}
          </div>
        </section>

        <section className="flex flex-1 flex-col gap-3 overflow-hidden rounded-2xl border border-border/70 bg-card p-4 shadow-sm">
          <div className="flex items-center justify-between">
            <div>
              <p className="mono text-[10px] uppercase tracking-[0.18em] text-muted-foreground">
                Friends
              </p>
              <h3 className="text-sm font-semibold">
                好友 {friends.length > 0 ? <span className="text-muted-foreground">· {friends.length}</span> : null}
              </h3>
            </div>
            <Dialog open={groupDialogOpen} onOpenChange={setGroupDialogOpen}>
              <DialogTrigger asChild>
                <Button variant="ghost" size="sm" disabled={friends.length < 2}>
                  <Users size={12} />
                  新建群
                </Button>
              </DialogTrigger>
              <DialogContent>
                <DialogHeader>
                  <DialogTitle>创建群聊</DialogTitle>
                  <DialogDescription>
                    请从好友列表中至少选择 2 位成员。
                  </DialogDescription>
                </DialogHeader>
                <div className="flex flex-col gap-3">
                  <div className="flex flex-col gap-2">
                    <Label>群名称</Label>
                    <Input
                      value={groupTitle}
                      onChange={(e) => setGroupTitle(e.target.value)}
                      placeholder="例如:基础设施答疑"
                    />
                  </div>
                  <ScrollArea className="max-h-64">
                    <div className="flex flex-col gap-1 pr-2">
                      {friends.map((friend) => {
                        const checked = selectedGroupMembers.includes(friend.account_id)
                        return (
                          <label
                            key={friend.account_id}
                            className={cn(
                              'flex cursor-pointer items-center gap-3 rounded-lg border border-border/60 px-3 py-2 transition-colors',
                              checked ? 'border-primary/40 bg-primary/5' : 'hover:bg-muted/60',
                            )}
                          >
                            <input
                              type="checkbox"
                              checked={checked}
                              onChange={() => toggleGroupMember(friend.account_id)}
                              className="size-4 accent-primary"
                            />
                            <div>
                              <div className="text-sm font-medium">{friend.display_name}</div>
                              <div className="mono text-[11px] text-muted-foreground">
                                @{friend.minos_id}
                              </div>
                            </div>
                          </label>
                        )
                      })}
                    </div>
                  </ScrollArea>
                </div>
                <DialogFooter>
                  <Button variant="secondary" onClick={() => setGroupDialogOpen(false)}>
                    取消
                  </Button>
                  <Button
                    disabled={groupBusy || selectedGroupMembers.length < 2}
                    onClick={handleCreateGroup}
                  >
                    {groupBusy ? '创建中…' : '创建'}
                  </Button>
                </DialogFooter>
              </DialogContent>
            </Dialog>
          </div>
          <ScrollArea className="flex-1">
            <div className="flex flex-col gap-1 pr-1">
              {friends.length === 0 ? (
                <p className="py-4 text-center text-xs text-muted-foreground">暂无好友</p>
              ) : (
                friends.map((friend) => (
                  <button
                    key={friend.account_id}
                    type="button"
                    onClick={() => openDirect(friend)}
                    className="flex items-center justify-between rounded-lg border border-transparent bg-background/40 px-3 py-2 text-left transition-colors hover:border-border hover:bg-muted/60"
                  >
                    <div className="min-w-0">
                      <div className="truncate text-sm font-medium">{friend.display_name}</div>
                      <div className="mono text-[11px] text-muted-foreground">
                        @{friend.minos_id}
                      </div>
                    </div>
                    <MessageSquareMore size={14} className="text-muted-foreground" />
                  </button>
                ))
              )}
            </div>
          </ScrollArea>
        </section>
      </aside>

      {/* Center — conversations */}
      <section className="flex w-80 shrink-0 flex-col gap-4">
        <section className="flex flex-col rounded-2xl border border-border/70 bg-card p-4 shadow-sm">
          <div>
            <p className="mono text-[10px] uppercase tracking-[0.18em] text-muted-foreground">
              Requests
            </p>
            <h3 className="text-sm font-semibold">好友请求</h3>
          </div>
          <div className="mt-3 flex flex-col gap-2">
            {pendingIncoming.length === 0 && pendingOutgoing.length === 0 ? (
              <p className="rounded-lg border border-dashed border-border/70 px-3 py-4 text-center text-xs text-muted-foreground">
                <UserPlus size={14} className="mx-auto mb-1 text-muted-foreground" />
                暂无待处理请求
              </p>
            ) : (
              <>
                {pendingIncoming.map((r) => (
                  <div
                    key={r.request_id}
                    className="flex items-center justify-between rounded-lg border border-border/70 bg-background/40 px-3 py-2"
                  >
                    <div className="min-w-0">
                      <div className="truncate text-sm font-medium">{r.from.display_name}</div>
                      <div className="mono text-[11px] text-muted-foreground">
                        @{r.from.minos_id}
                      </div>
                    </div>
                    <div className="flex gap-1">
                      <Button
                        variant="ghost"
                        size="icon"
                        className="size-7"
                        onClick={() => handleResolve(r.request_id, 'reject')}
                        title="拒绝"
                      >
                        <X size={14} />
                      </Button>
                      <Button
                        size="icon"
                        className="size-7"
                        onClick={() => handleResolve(r.request_id, 'accept')}
                        title="接受"
                      >
                        <Check size={14} />
                      </Button>
                    </div>
                  </div>
                ))}
                {pendingOutgoing.map((r) => (
                  <div
                    key={r.request_id}
                    className="flex items-center justify-between rounded-lg border border-border/70 bg-background/40 px-3 py-2"
                  >
                    <div className="min-w-0">
                      <div className="truncate text-sm font-medium">{r.to.display_name}</div>
                      <div className="mono text-[11px] text-muted-foreground">
                        @{r.to.minos_id} · 已发送
                      </div>
                    </div>
                    <Badge variant="outline">等待对方</Badge>
                  </div>
                ))}
              </>
            )}
          </div>
        </section>

        <section className="flex flex-1 flex-col overflow-hidden rounded-2xl border border-border/70 bg-card shadow-sm">
          <div className="border-b border-border/60 px-4 py-3">
            <p className="mono text-[10px] uppercase tracking-[0.18em] text-muted-foreground">
              Conversations
            </p>
            <h3 className="text-sm font-semibold">对话</h3>
          </div>
          <ScrollArea className="flex-1">
            <div className="flex flex-col gap-1 p-2">
              {bootBusy && conversations.length === 0 ? (
                <p className="py-4 text-center text-xs text-muted-foreground">加载中…</p>
              ) : conversations.length === 0 ? (
                <p className="py-4 text-center text-xs text-muted-foreground">暂无对话</p>
              ) : (
                conversations.map((c) => {
                  const active = selectedConversationId === c.conversation_id
                  return (
                    <button
                      key={c.conversation_id}
                      type="button"
                      onClick={() => setSelectedConversationId(c.conversation_id)}
                      className={cn(
                        'flex flex-col gap-1 rounded-xl px-3 py-2.5 text-left transition-colors',
                        active
                          ? 'bg-primary/10 ring-1 ring-inset ring-primary/30'
                          : 'hover:bg-muted/60',
                      )}
                    >
                      <div className="flex items-center justify-between gap-2">
                        <span className="truncate text-sm font-medium">{c.title}</span>
                        <span className="shrink-0 text-[10px] text-muted-foreground">
                          {formatRelative(c.last_message_at_ms)}
                        </span>
                      </div>
                      <p className="line-clamp-1 text-xs text-muted-foreground">
                        {c.last_message_preview ?? '暂无消息'}
                      </p>
                      {c.unread_count > 0 || c.unread_mention_count > 0 ? (
                        <div className="flex items-center gap-1">
                          {c.unread_mention_count > 0 ? (
                            <Badge variant="destructive">@{c.unread_mention_count}</Badge>
                          ) : null}
                          {c.unread_count > 0 ? (
                            <Badge variant="default">{c.unread_count}</Badge>
                          ) : null}
                        </div>
                      ) : null}
                    </button>
                  )
                })
              )}
            </div>
          </ScrollArea>
        </section>
      </section>

      {/* Right — active conversation */}
      <main className="flex flex-1 flex-col overflow-hidden rounded-2xl border border-border/70 bg-card shadow-sm">
        {!activeConversation ? (
          <div className="flex h-full flex-col items-center justify-center gap-3 p-10 text-center text-muted-foreground">
            <div className="flex size-14 items-center justify-center rounded-2xl bg-primary/10 text-primary">
              <MessageSquareMore size={24} />
            </div>
            <p className="text-sm">选择一个对话开始聊天,或新建群聊。</p>
          </div>
        ) : (
          <>
            <div className="flex items-center justify-between border-b border-border/60 px-6 py-4">
              <div className="min-w-0">
                <p className="mono text-[10px] uppercase tracking-[0.18em] text-muted-foreground">
                  {activeConversation.kind === 'group' ? 'Group chat' : 'Direct message'}
                </p>
                <h2 className="truncate text-base font-semibold">{activeConversation.title}</h2>
              </div>
              <div className="flex items-center gap-2">
                <Badge variant="outline">{activeConversation.member_count} 成员</Badge>
                <Dialog open={mentionOpen} onOpenChange={setMentionOpen}>
                  <DialogTrigger asChild>
                    <Button variant="ghost" size="sm" disabled={activeMembers.length === 0}>
                      <AtSign size={12} />
                      提及
                    </Button>
                  </DialogTrigger>
                  <DialogContent>
                    <DialogHeader>
                      <DialogTitle>插入提及</DialogTitle>
                      <DialogDescription>选择一位成员,将 @minosId 插入输入框。</DialogDescription>
                    </DialogHeader>
                    <ScrollArea className="max-h-72">
                      <div className="flex flex-col gap-1 pr-1">
                        {activeMembers.map((m) => (
                          <button
                            key={m.account_id}
                            type="button"
                            onClick={() => insertMention(m)}
                            className="flex items-center justify-between rounded-lg border border-border/60 bg-background/40 px-3 py-2 text-left transition-colors hover:bg-muted/60"
                          >
                            <div>
                              <div className="text-sm font-medium">{m.display_name}</div>
                              <div className="mono text-[11px] text-muted-foreground">
                                @{m.minos_id}
                              </div>
                            </div>
                          </button>
                        ))}
                      </div>
                    </ScrollArea>
                  </DialogContent>
                </Dialog>
              </div>
            </div>

            <div className="flex-1 overflow-y-auto px-6 py-4" ref={messageScrollRef}>
              <AnimatePresence initial={false}>
                {activeMessages.length === 0 ? (
                  <div className="mt-10 text-center text-xs text-muted-foreground">
                    还没有消息,先打个招呼吧 👋
                  </div>
                ) : (
                  <div className="flex flex-col gap-4">
                    {activeMessages.map((m) => {
                      const isMine = senderIsMine(m.sender, activeSession.accountId)
                      const mentionsMe =
                        !isMine &&
                        (m.mentioned_account_ids ?? []).includes(activeSession.accountId)
                      return (
                        <motion.article
                          key={m.message_id}
                          initial={{ opacity: 0, y: 8 }}
                          animate={{ opacity: 1, y: 0 }}
                          className={cn('flex flex-col gap-1', isMine ? 'items-end' : 'items-start')}
                        >
                          <div className="flex items-center gap-2 text-[11px] text-muted-foreground">
                            <span className="font-medium text-foreground">
                              {isMine ? '我' : m.sender.display_name}
                            </span>
                            {!isMine ? (
                              <span className="mono">@{senderHandle(m.sender)}</span>
                            ) : null}
                            <span>{formatClock(m.created_at_ms)}</span>
                          </div>
                          {m.reply_to ? (
                            <div className="mb-1 max-w-[70%] rounded-lg border-l-2 border-primary/60 bg-muted/50 px-3 py-1.5 text-xs text-muted-foreground">
                              <div className="font-medium text-foreground">
                                {m.reply_to.sender.display_name}
                              </div>
                              <div className="line-clamp-2">{m.reply_to.text}</div>
                            </div>
                          ) : null}
                          <div
                            className={cn(
                              'max-w-[70%] rounded-2xl px-4 py-2.5 text-sm leading-relaxed whitespace-pre-wrap shadow-sm',
                              isMine
                                ? 'bg-primary text-primary-foreground rounded-tr-sm'
                                : 'bg-muted text-foreground rounded-tl-sm',
                              mentionsMe && 'ring-2 ring-warning/60',
                              m.recalled_at_ms &&
                                'bg-muted/60 text-muted-foreground italic',
                            )}
                          >
                            {m.recalled_at_ms ? '此消息已被撤回' : m.text}
                          </div>
                          {!m.recalled_at_ms ? (
                            <div className="flex gap-1 opacity-70 hover:opacity-100">
                              <Button
                                variant="ghost"
                                size="sm"
                                className="h-6 px-2 text-[11px]"
                                onClick={() => setReplyTarget(m)}
                              >
                                回复
                              </Button>
                              {isMine ? (
                                <Button
                                  variant="ghost"
                                  size="sm"
                                  className="h-6 px-2 text-[11px]"
                                  disabled={recallingId === m.message_id}
                                  onClick={() => handleRecall(m)}
                                >
                                  <Undo2 size={12} />
                                  {recallingId === m.message_id ? '撤回中…' : '撤回'}
                                </Button>
                              ) : null}
                            </div>
                          ) : null}
                        </motion.article>
                      )
                    })}
                  </div>
                )}
              </AnimatePresence>
            </div>

            <div className="border-t border-border/60 bg-background/50 p-4 backdrop-blur-md">
              {replyTarget ? (
                <div className="mb-2 flex items-center justify-between rounded-lg border border-primary/30 bg-primary/5 px-3 py-2 text-xs">
                  <div className="min-w-0">
                    <div className="font-medium text-foreground">
                      回复 {replyTarget.sender.display_name}
                    </div>
                    <div className="line-clamp-1 text-muted-foreground">{replyTarget.text}</div>
                  </div>
                  <Button
                    variant="ghost"
                    size="icon"
                    className="size-7"
                    onClick={() => setReplyTarget(null)}
                  >
                    <X size={12} />
                  </Button>
                </div>
              ) : null}
              <div className="relative flex items-end gap-2 rounded-2xl border border-border bg-card p-2 shadow-sm transition-all focus-within:border-primary/50 focus-within:ring-4 focus-within:ring-primary/10">
                <Textarea
                  value={composer}
                  onChange={(e) => setComposer(e.target.value)}
                  placeholder="输入消息…"
                  className="max-h-[220px] min-h-[44px] resize-none border-0 px-3 py-3 shadow-none focus-visible:ring-0"
                  onKeyDown={(e) => {
                    if (e.key === 'Enter' && !e.shiftKey) {
                      e.preventDefault()
                      handleSend()
                    }
                  }}
                />
                <Button
                  size="icon"
                  className="size-10 rounded-xl"
                  disabled={!composer.trim() || sendBusy}
                  onClick={handleSend}
                >
                  {sendBusy ? (
                    <RefreshCw size={16} className="animate-spin" />
                  ) : (
                    <Send size={16} />
                  )}
                </Button>
              </div>
            </div>
          </>
        )}
      </main>
    </div>
  )
}
