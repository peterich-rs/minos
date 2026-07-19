import { useEffect, useState } from 'react'
import { toast } from 'sonner'
import {
  Check,
  LogOut,
  Mail,
  RefreshCw,
  ShieldCheck,
  Sparkles,
  UserPlus,
  UserRound,
  X,
} from 'lucide-react'

import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { ScrollArea } from '@/components/ui/scroll-area'
import {
  type FriendRequestsResponse,
  type MyProfileResponse,
  acceptFriendRequest,
  changePassword,
  getMyProfile,
  listFriendRequests,
  rejectFriendRequest,
  runWithSessionRefresh,
  setMyDisplayName,
  setMyMinosId,
} from '@/lib/minos'
import { useAppStore } from '@/lib/store'

export function ProfileWorkspace() {
  const { deviceId, session, setSession, logout } = useAppStore()
  const activeSession = session!

  const [profile, setProfile] = useState<MyProfileResponse | null>(null)
  const [requests, setRequests] = useState<FriendRequestsResponse>({ incoming: [], outgoing: [] })
  const [loading, setLoading] = useState(true)
  const [refreshing, setRefreshing] = useState(false)

  const [displayDraft, setDisplayDraft] = useState('')
  const [minosIdDraft, setMinosIdDraft] = useState('')
  const [savingDisplay, setSavingDisplay] = useState(false)
  const [savingMinosId, setSavingMinosId] = useState(false)

  const [currentPw, setCurrentPw] = useState('')
  const [newPw, setNewPw] = useState('')
  const [confirmPw, setConfirmPw] = useState('')
  const [changingPw, setChangingPw] = useState(false)

  async function reload() {
    try {
      const [p, r] = await runWithSessionRefresh(activeSession, deviceId, setSession, (current) =>
        Promise.all([
          getMyProfile(deviceId, current.accessToken),
          listFriendRequests(deviceId, current.accessToken),
        ]),
      )
      setProfile(p)
      setDisplayDraft(p.display_name?.trim() ?? '')
      setMinosIdDraft(p.minos_id)
      setRequests(r)
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e))
    }
  }

  useEffect(() => {
    let cancelled = false
    // Initial load of profile + friend requests. setLoading flip is
    // wrapped in an effect because it is bound to a network fetch, not
    // a render-derived value.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    void reload().finally(() => {
      if (!cancelled) setLoading(false)
    })
    return () => {
      cancelled = true
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  async function handleRefresh() {
    setRefreshing(true)
    await reload()
    setRefreshing(false)
  }

  async function handleSaveDisplayName() {
    const trimmed = displayDraft.trim()
    if (!profile) return
    if (trimmed === (profile.display_name ?? '').trim()) {
      toast('昵称未改动')
      return
    }
    try {
      setSavingDisplay(true)
      const next = await runWithSessionRefresh(activeSession, deviceId, setSession, (current) =>
        setMyDisplayName(deviceId, current.accessToken, trimmed || null),
      )
      setProfile(next)
      setDisplayDraft(next.display_name?.trim() ?? '')
      toast.success('昵称已更新')
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e))
    } finally {
      setSavingDisplay(false)
    }
  }

  async function handleSaveMinosId() {
    if (!profile) return
    const trimmed = minosIdDraft.trim()
    if (trimmed === profile.minos_id) {
      toast('Minos ID 未改动')
      return
    }
    try {
      setSavingMinosId(true)
      const next = await runWithSessionRefresh(activeSession, deviceId, setSession, (current) =>
        setMyMinosId(deviceId, current.accessToken, trimmed),
      )
      setProfile(next)
      setMinosIdDraft(next.minos_id)
      toast.success('Minos ID 已更新')
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e))
    } finally {
      setSavingMinosId(false)
    }
  }

  async function handleChangePassword() {
    if (newPw.length < 8) {
      toast.error('新密码至少 8 个字符')
      return
    }
    if (newPw !== confirmPw) {
      toast.error('两次输入的新密码不一致')
      return
    }
    try {
      setChangingPw(true)
      await runWithSessionRefresh(activeSession, deviceId, setSession, (current) =>
        changePassword(deviceId, current.accessToken, currentPw, newPw),
      )
      toast.success('密码已更新,其他设备需重新登录')
      setCurrentPw('')
      setNewPw('')
      setConfirmPw('')
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e))
    } finally {
      setChangingPw(false)
    }
  }

  async function resolveRequest(requestId: string, action: 'accept' | 'reject') {
    try {
      if (action === 'accept') {
        await runWithSessionRefresh(activeSession, deviceId, setSession, (current) =>
          acceptFriendRequest(deviceId, current.accessToken, requestId),
        )
        toast.success('已接受')
      } else {
        await runWithSessionRefresh(activeSession, deviceId, setSession, (current) =>
          rejectFriendRequest(deviceId, current.accessToken, requestId),
        )
        toast('已拒绝')
      }
      await reload()
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e))
    }
  }

  const pendingIncoming = requests.incoming.filter((r) => r.status === 'pending')
  const pendingOutgoing = requests.outgoing.filter((r) => r.status === 'pending')

  if (loading) {
    return (
      <div className="flex h-full items-center justify-center">
        <RefreshCw size={24} className="animate-spin text-muted-foreground" />
      </div>
    )
  }

  return (
    <ScrollArea className="h-full">
      <div className="mx-auto flex max-w-4xl flex-col gap-4 p-6">
        {/* Hero */}
        <Card className="overflow-hidden">
          <div className="relative bg-gradient-to-br from-primary/10 via-background to-background p-8">
            <div className="flex items-start gap-5">
              <div className="flex size-20 items-center justify-center rounded-3xl bg-primary text-primary-foreground shadow-lg ring-4 ring-background">
                <UserRound size={32} />
              </div>
              <div className="flex-1 min-w-0">
                <p className="mono text-[10px] uppercase tracking-[0.18em] text-muted-foreground">
                  Browser admin
                </p>
                <h2 className="text-2xl font-bold tracking-tight">
                  {profile?.display_name?.trim() || profile?.minos_id || activeSession.email}
                </h2>
                <div className="mt-2 flex flex-wrap items-center gap-2">
                  <Badge variant="outline" className="mono">
                    @{profile?.minos_id}
                  </Badge>
                  <Badge variant="secondary">
                    <Mail size={12} />
                    {activeSession.email}
                  </Badge>
                </div>
              </div>
              <Button variant="outline" size="sm" onClick={handleRefresh} disabled={refreshing}>
                <RefreshCw size={14} className={refreshing ? 'animate-spin' : ''} />
                刷新
              </Button>
            </div>
          </div>
        </Card>

        {/* Display name */}
        <Card>
          <CardHeader>
            <CardTitle>个人资料</CardTitle>
            <CardDescription>
              昵称和 Minos ID 会展示在好友、群聊以及 @ 提及中。
            </CardDescription>
          </CardHeader>
          <CardContent className="flex flex-col gap-4">
            <div className="grid grid-cols-1 gap-2 md:grid-cols-[1fr_auto]">
              <div className="flex flex-col gap-1.5">
                <Label>昵称</Label>
                <Input
                  value={displayDraft}
                  onChange={(e) => setDisplayDraft(e.target.value)}
                  placeholder="留空则使用邮箱前缀"
                />
              </div>
              <Button
                className="self-end"
                onClick={handleSaveDisplayName}
                disabled={savingDisplay}
              >
                {savingDisplay ? '保存中…' : '保存昵称'}
              </Button>
            </div>
            <div className="grid grid-cols-1 gap-2 md:grid-cols-[1fr_auto]">
              <div className="flex flex-col gap-1.5">
                <Label>Minos ID</Label>
                <Input
                  value={minosIdDraft}
                  onChange={(e) => setMinosIdDraft(e.target.value)}
                  placeholder="6-24 个字母或数字"
                  className="mono"
                />
              </div>
              <Button
                variant="secondary"
                className="self-end"
                onClick={handleSaveMinosId}
                disabled={savingMinosId}
              >
                {savingMinosId ? '保存中…' : '保存 ID'}
              </Button>
            </div>
          </CardContent>
        </Card>

        {/* Password */}
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <ShieldCheck size={18} className="text-primary" />
              修改密码
            </CardTitle>
            <CardDescription>
              修改密码后,其他已登录设备会被强制下线,需要重新登录。
            </CardDescription>
          </CardHeader>
          <CardContent className="flex flex-col gap-3">
            <div className="grid grid-cols-1 gap-3 md:grid-cols-3">
              <div className="flex flex-col gap-1.5">
                <Label>当前密码</Label>
                <Input
                  type="password"
                  value={currentPw}
                  onChange={(e) => setCurrentPw(e.target.value)}
                />
              </div>
              <div className="flex flex-col gap-1.5">
                <Label>新密码</Label>
                <Input type="password" value={newPw} onChange={(e) => setNewPw(e.target.value)} />
              </div>
              <div className="flex flex-col gap-1.5">
                <Label>确认新密码</Label>
                <Input
                  type="password"
                  value={confirmPw}
                  onChange={(e) => setConfirmPw(e.target.value)}
                />
              </div>
            </div>
            <div className="flex items-center justify-between">
              <p className="text-xs text-muted-foreground">
                新密码至少 8 个字符。建议包含字母和数字。
              </p>
              <Button
                onClick={handleChangePassword}
                disabled={changingPw || !currentPw || !newPw || !confirmPw}
              >
                {changingPw ? '更新中…' : '更新密码'}
              </Button>
            </div>
          </CardContent>
        </Card>

        {/* Friend requests */}
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <UserPlus size={18} className="text-primary" />
              未处理的好友请求
              {pendingIncoming.length > 0 ? (
                <Badge variant="destructive">{pendingIncoming.length}</Badge>
              ) : null}
            </CardTitle>
            <CardDescription>
              你发送和收到的好友请求状态概览。
            </CardDescription>
          </CardHeader>
          <CardContent className="flex flex-col gap-4">
            <div>
              <h4 className="mono mb-2 text-[10px] uppercase tracking-widest text-muted-foreground">
                Incoming · {pendingIncoming.length}
              </h4>
              {pendingIncoming.length === 0 ? (
                <p className="rounded-lg border border-dashed border-border/70 px-3 py-4 text-center text-xs text-muted-foreground">
                  没有待处理的来访请求
                </p>
              ) : (
                <div className="flex flex-col gap-2">
                  {pendingIncoming.map((r) => (
                    <div
                      key={r.request_id}
                      className="flex items-center justify-between rounded-lg border border-border/70 bg-background/40 px-4 py-2.5"
                    >
                      <div className="min-w-0">
                        <div className="truncate text-sm font-medium">{r.from.display_name}</div>
                        <div className="mono text-xs text-muted-foreground">
                          @{r.from.minos_id}
                        </div>
                      </div>
                      <div className="flex gap-2">
                        <Button
                          variant="outline"
                          size="sm"
                          onClick={() => resolveRequest(r.request_id, 'reject')}
                        >
                          <X size={12} />
                          拒绝
                        </Button>
                        <Button size="sm" onClick={() => resolveRequest(r.request_id, 'accept')}>
                          <Check size={12} />
                          接受
                        </Button>
                      </div>
                    </div>
                  ))}
                </div>
              )}
            </div>
            <div>
              <h4 className="mono mb-2 text-[10px] uppercase tracking-widest text-muted-foreground">
                Outgoing · {pendingOutgoing.length}
              </h4>
              {pendingOutgoing.length === 0 ? (
                <p className="rounded-lg border border-dashed border-border/70 px-3 py-4 text-center text-xs text-muted-foreground">
                  没有等待对方回复的请求
                </p>
              ) : (
                <div className="flex flex-col gap-2">
                  {pendingOutgoing.map((r) => (
                    <div
                      key={r.request_id}
                      className="flex items-center justify-between rounded-lg border border-border/70 bg-background/40 px-4 py-2.5"
                    >
                      <div className="min-w-0">
                        <div className="truncate text-sm font-medium">{r.to.display_name}</div>
                        <div className="mono text-xs text-muted-foreground">@{r.to.minos_id}</div>
                      </div>
                      <Badge variant="outline">
                        <Sparkles size={12} />
                        等待对方
                      </Badge>
                    </div>
                  ))}
                </div>
              )}
            </div>
          </CardContent>
        </Card>

        {/* Sign out */}
        <Card>
          <CardContent className="flex flex-col gap-2 pt-6 sm:flex-row sm:items-center sm:justify-between">
            <div>
              <h3 className="text-sm font-semibold">退出当前账户</h3>
              <p className="mt-0.5 text-xs text-muted-foreground">
                会清除浏览器端的登录凭证,其他设备不受影响。
              </p>
            </div>
            <Button variant="outline" onClick={logout}>
              <LogOut size={14} />
              退出登录
            </Button>
          </CardContent>
        </Card>
      </div>
    </ScrollArea>
  )
}
