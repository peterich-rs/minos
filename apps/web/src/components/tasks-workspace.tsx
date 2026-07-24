import { useEffect, useState } from 'react'
import { motion } from 'framer-motion'
import { toast } from 'sonner'
import {
  Activity,
  Bot,
  CheckCircle2,
  Clock3,
  Cpu,
  Hash,
  Pencil,
  Plus,
  Sparkles,
  Star,
  Trash2,
  XCircle,
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
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Textarea } from '@/components/ui/textarea'
import {
  type AgentProfile,
  type AgentReasoningEffort,
  type AgentWorkspaceState,
  createAgentProfileId,
  loadAgentWorkspace,
  normalizeAgentWorkspace,
  saveAgentWorkspace,
} from '@/lib/agent-profiles'
import type { AgentName, HostSummary, SessionSummary } from '@/lib/minos'
import { useAppStore } from '@/lib/store'
import { cn } from '@/lib/utils'

const AGENT_OPTIONS: AgentName[] = ['codex', 'claude', 'gemini']
const REASONING_OPTIONS: AgentReasoningEffort[] = ['low', 'medium', 'high']

function agentLabel(a: AgentName): string {
  return a[0].toUpperCase() + a.slice(1)
}

function defaultModel(agent: AgentName): string {
  switch (agent) {
    case 'codex':
      return 'GPT-5.5'
    case 'claude':
      return 'Claude Sonnet 4'
    case 'gemini':
      return 'Gemini 2.5 Pro'
    default:
      return 'GPT-5.5'
  }
}

function hostLabel(hosts: HostSummary[], id: string | null | undefined): string {
  if (!id) return '跟随当前 Runtime'
  return hosts.find((h) => h.host_device_id === id)?.host_display_name ?? id
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

type ThreadBucket = 'running' | 'done' | 'error'

function bucketOf(t: SessionSummary): ThreadBucket {
  if (!t.ended_at_ms) return 'running'
  if (t.end_reason?.kind === 'crashed' || t.end_reason?.kind === 'timeout') return 'error'
  return 'done'
}

type ProfileDraft = {
  name: string
  description: string
  runtimeAgent: AgentName
  model: string
  reasoningEffort: AgentReasoningEffort
  hostDeviceId: string | null
}

function createDraft(
  hosts: HostSummary[],
  activeHost: string | null,
  profile?: AgentProfile | null,
): ProfileDraft {
  if (profile) {
    return {
      name: profile.name,
      description: profile.description,
      runtimeAgent: profile.runtimeAgent,
      model: profile.model,
      reasoningEffort: profile.reasoningEffort,
      hostDeviceId: profile.hostDeviceId ?? null,
    }
  }
  return {
    name: '新预设',
    description: '',
    runtimeAgent: 'codex',
    model: defaultModel('codex'),
    reasoningEffort: 'medium',
    hostDeviceId: activeHost ?? hosts[0]?.host_device_id ?? null,
  }
}

function commitDraft(
  draft: ProfileDraft,
  existing: AgentProfile | null,
  hosts: HostSummary[],
): AgentProfile {
  const now = Date.now()
  const hostDisplay = draft.hostDeviceId ? hostLabel(hosts, draft.hostDeviceId) : null
  return {
    id: existing?.id ?? createAgentProfileId(),
    name: draft.name.trim() || 'Agent',
    description: draft.description.trim(),
    runtimeAgent: draft.runtimeAgent,
    model: draft.model.trim() || defaultModel(draft.runtimeAgent),
    reasoningEffort: draft.reasoningEffort,
    environmentVariables: existing?.environmentVariables ?? [],
    hostDeviceId: draft.hostDeviceId ?? null,
    hostDisplayName: draft.hostDeviceId ? hostDisplay : null,
    createdAtMs: existing?.createdAtMs ?? now,
    updatedAtMs: now,
  }
}

export function TasksWorkspace() {
  const {
    sessions,
    hosts,
    activeHost,
    setSelectedThreadId,
    setRoute,
  } = useAppStore()
  const [workspace, setWorkspace] = useState<AgentWorkspaceState>(loadAgentWorkspace)
  const [selectedProfileId, setSelectedProfileId] = useState<string | null>(
    workspace.preferredProfileId ?? workspace.profiles[0]?.id ?? null,
  )
  const [dialogOpen, setDialogOpen] = useState(false)
  const [editingId, setEditingId] = useState<string | null>(null)
  const [draft, setDraft] = useState<ProfileDraft>(() => createDraft(hosts, activeHost))

  useEffect(() => {
    saveAgentWorkspace(workspace)
  }, [workspace])

  const buckets = sessions.reduce(
    (acc, thread) => {
      acc[bucketOf(thread)].push(thread)
      return acc
    },
    { running: [] as SessionSummary[], done: [] as SessionSummary[], error: [] as SessionSummary[] },
  )
  const selectedProfile =
    workspace.profiles.find((p) => p.id === selectedProfileId) ?? null

  function commitWorkspace(next: AgentWorkspaceState) {
    const normalized = normalizeAgentWorkspace(next)
    setWorkspace(normalized)
    setSelectedProfileId((current) => {
      if (current && normalized.profiles.some((p) => p.id === current)) return current
      return normalized.preferredProfileId ?? normalized.profiles[0]?.id ?? null
    })
  }

  function openCreate() {
    setEditingId(null)
    setDraft(createDraft(hosts, activeHost))
    setDialogOpen(true)
  }

  function openEdit(profile: AgentProfile) {
    setEditingId(profile.id)
    setDraft(createDraft(hosts, activeHost, profile))
    setDialogOpen(true)
  }

  function saveProfile() {
    const existing = workspace.profiles.find((p) => p.id === editingId) ?? null
    const next = commitDraft(draft, existing, hosts)
    if (existing) {
      commitWorkspace({
        ...workspace,
        profiles: workspace.profiles.map((p) => (p.id === existing.id ? next : p)),
      })
    } else {
      commitWorkspace({
        ...workspace,
        profiles: [...workspace.profiles, next],
        preferredProfileId: workspace.preferredProfileId ?? next.id,
      })
    }
    setDialogOpen(false)
    toast.success(existing ? '已更新预设' : '已创建预设')
  }

  function deleteProfile(profileId: string) {
    const remaining = workspace.profiles.filter((p) => p.id !== profileId)
    commitWorkspace({
      ...workspace,
      profiles: remaining,
      preferredProfileId:
        workspace.preferredProfileId === profileId
          ? remaining[0]?.id ?? null
          : workspace.preferredProfileId,
    })
    toast('已删除预设')
  }

  function setPreferred(profileId: string) {
    commitWorkspace({ ...workspace, preferredProfileId: profileId })
  }

  function openThread(thread: SessionSummary) {
    setSelectedThreadId(thread.session_id)
    setRoute('chat')
  }

  return (
    <div className="flex h-full flex-col gap-4 overflow-y-auto p-4">
      {/* Threads overview */}
      <Card>
        <CardHeader className="pb-3">
          <CardTitle>任务看板</CardTitle>
          <CardDescription>
            所有 Agent 回合按状态分组,点击任意卡片跳转到聊天页查看详情。
          </CardDescription>
        </CardHeader>
        <CardContent className="grid grid-cols-1 gap-4 pt-0 md:grid-cols-3">
          <ThreadBucketColumn
            title="进行中"
            bucket="running"
            tone="primary"
            sessions={buckets.running}
            onOpen={openThread}
            empty="没有正在运行的 Agent"
          />
          <ThreadBucketColumn
            title="已完成"
            bucket="done"
            tone="success"
            sessions={buckets.done}
            onOpen={openThread}
            empty="没有已完成的记录"
          />
          <ThreadBucketColumn
            title="异常"
            bucket="error"
            tone="destructive"
            sessions={buckets.error}
            onOpen={openThread}
            empty="没有异常结束的任务"
          />
        </CardContent>
      </Card>

      {/* Profiles */}
      <Card className="flex flex-col">
        <CardHeader className="flex-row items-center justify-between space-y-0 pb-3">
          <div>
            <CardTitle>Agent 预设</CardTitle>
            <CardDescription>
              每个预设保存在本地,用来快速切换 Runtime / 模型 / 绑定主机。
            </CardDescription>
          </div>
          <Button size="sm" onClick={openCreate}>
            <Plus size={14} />
            新建预设
          </Button>
        </CardHeader>
        <CardContent className="grid grid-cols-1 gap-4 pt-0 lg:grid-cols-[320px_1fr]">
          <ScrollArea className="max-h-[420px] rounded-xl border border-border/70 bg-background/40 p-2">
            <div className="flex flex-col gap-1">
              {workspace.profiles.length === 0 ? (
                <p className="px-3 py-6 text-center text-xs text-muted-foreground">
                  还没有预设,点击右上角 "新建预设"。
                </p>
              ) : (
                workspace.profiles.map((profile) => {
                  const active = selectedProfileId === profile.id
                  return (
                    <button
                      key={profile.id}
                      type="button"
                      onClick={() => setSelectedProfileId(profile.id)}
                      className={cn(
                        'flex items-center justify-between gap-2 rounded-xl px-3 py-2.5 text-left transition-colors',
                        active ? 'bg-primary/10 ring-1 ring-inset ring-primary/30' : 'hover:bg-muted/60',
                      )}
                    >
                      <div className="min-w-0">
                        <div className="flex items-center gap-2">
                          <span className="truncate text-sm font-medium">{profile.name}</span>
                          {workspace.preferredProfileId === profile.id ? (
                            <Badge variant="secondary" className="shrink-0">
                              默认
                            </Badge>
                          ) : null}
                        </div>
                        <div className="mono text-[11px] text-muted-foreground">
                          {agentLabel(profile.runtimeAgent)} · {profile.model}
                        </div>
                      </div>
                    </button>
                  )
                })
              )}
            </div>
          </ScrollArea>

          <div className="rounded-xl border border-border/70 bg-background/40 p-5">
            {!selectedProfile ? (
              <div className="flex h-full flex-col items-center justify-center gap-3 text-center text-muted-foreground">
                <div className="flex size-12 items-center justify-center rounded-2xl bg-primary/10 text-primary">
                  <Sparkles size={20} />
                </div>
                <p className="text-sm">选择一个预设查看详情,或新建一个开始。</p>
              </div>
            ) : (
              <div className="flex flex-col gap-4">
                <div className="flex items-start justify-between gap-2">
                  <div>
                    <h3 className="text-lg font-semibold">{selectedProfile.name}</h3>
                    <p className="mt-1 text-sm text-muted-foreground">
                      {selectedProfile.description || '尚未填写描述。'}
                    </p>
                  </div>
                  <div className="flex shrink-0 gap-1">
                    <Button variant="ghost" size="sm" onClick={() => openEdit(selectedProfile)}>
                      <Pencil size={12} />
                      编辑
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => deleteProfile(selectedProfile.id)}
                    >
                      <Trash2 size={12} />
                      删除
                    </Button>
                  </div>
                </div>
                <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
                  <SummaryStat
                    icon={<Bot size={14} />}
                    label="Runtime"
                    value={agentLabel(selectedProfile.runtimeAgent)}
                    hint={selectedProfile.model}
                  />
                  <SummaryStat
                    icon={<Activity size={14} />}
                    label="Reasoning"
                    value={selectedProfile.reasoningEffort}
                  />
                  <SummaryStat
                    icon={<Cpu size={14} />}
                    label="Host"
                    value={hostLabel(hosts, selectedProfile.hostDeviceId)}
                    hint={selectedProfile.hostDeviceId ?? '跟随当前 Runtime'}
                  />
                </div>
                <div className="flex gap-2">
                  <Button
                    variant="secondary"
                    size="sm"
                    disabled={workspace.preferredProfileId === selectedProfile.id}
                    onClick={() => setPreferred(selectedProfile.id)}
                  >
                    <Star size={12} />
                    设为默认
                  </Button>
                  <Button size="sm" onClick={() => setRoute('chat')}>
                    <Sparkles size={12} />
                    在聊天中使用
                  </Button>
                </div>
              </div>
            )}
          </div>
        </CardContent>
      </Card>

      <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
        <DialogContent>
          <form
            onSubmit={(e) => {
              e.preventDefault()
              saveProfile()
            }}
            className="flex flex-col gap-4"
          >
            <DialogHeader>
              <DialogTitle>{editingId ? '编辑预设' : '新建预设'}</DialogTitle>
              <DialogDescription>
                预设仅保存在浏览器本地,用来快速切换 Runtime 和模型。
              </DialogDescription>
            </DialogHeader>
            <div className="grid gap-4 sm:grid-cols-2">
              <div className="flex flex-col gap-1.5">
                <Label>名称</Label>
                <Input
                  value={draft.name}
                  onChange={(e) => setDraft((d) => ({ ...d, name: e.target.value }))}
                  placeholder="review-agent"
                />
              </div>
              <div className="flex flex-col gap-1.5">
                <Label>模型</Label>
                <Input
                  value={draft.model}
                  onChange={(e) => setDraft((d) => ({ ...d, model: e.target.value }))}
                  placeholder={defaultModel(draft.runtimeAgent)}
                />
              </div>
              <div className="flex flex-col gap-1.5 sm:col-span-2">
                <Label>描述</Label>
                <Textarea
                  value={draft.description}
                  onChange={(e) => setDraft((d) => ({ ...d, description: e.target.value }))}
                  rows={2}
                  placeholder="用一句话记录这个预设的用途。"
                />
              </div>
              <div className="flex flex-col gap-1.5">
                <Label>Runtime</Label>
                <div className="flex gap-2">
                  {AGENT_OPTIONS.map((agent) => (
                    <button
                      key={agent}
                      type="button"
                      onClick={() =>
                        setDraft((d) => ({ ...d, runtimeAgent: agent, model: defaultModel(agent) }))
                      }
                      className={cn(
                        'flex-1 rounded-lg border px-3 py-2 text-sm transition-colors',
                        draft.runtimeAgent === agent
                          ? 'border-primary/60 bg-primary/10 text-primary'
                          : 'border-border hover:bg-muted/60',
                      )}
                    >
                      {agentLabel(agent)}
                    </button>
                  ))}
                </div>
              </div>
              <div className="flex flex-col gap-1.5">
                <Label>Reasoning</Label>
                <div className="flex gap-2">
                  {REASONING_OPTIONS.map((r) => (
                    <button
                      key={r}
                      type="button"
                      onClick={() => setDraft((d) => ({ ...d, reasoningEffort: r }))}
                      className={cn(
                        'flex-1 rounded-lg border px-3 py-2 text-sm transition-colors',
                        draft.reasoningEffort === r
                          ? 'border-primary/60 bg-primary/10 text-primary'
                          : 'border-border hover:bg-muted/60',
                      )}
                    >
                      {r}
                    </button>
                  ))}
                </div>
              </div>
              <div className="flex flex-col gap-1.5 sm:col-span-2">
                <Label>绑定主机</Label>
                <div className="flex flex-wrap gap-2">
                  <button
                    type="button"
                    onClick={() => setDraft((d) => ({ ...d, hostDeviceId: null }))}
                    className={cn(
                      'rounded-lg border px-3 py-2 text-sm transition-colors',
                      draft.hostDeviceId == null
                        ? 'border-primary/60 bg-primary/10 text-primary'
                        : 'border-border hover:bg-muted/60',
                    )}
                  >
                    跟随当前 Runtime
                  </button>
                  {hosts.map((h) => (
                    <button
                      key={h.host_device_id}
                      type="button"
                      onClick={() => setDraft((d) => ({ ...d, hostDeviceId: h.host_device_id }))}
                      className={cn(
                        'rounded-lg border px-3 py-2 text-sm transition-colors',
                        draft.hostDeviceId === h.host_device_id
                          ? 'border-primary/60 bg-primary/10 text-primary'
                          : 'border-border hover:bg-muted/60',
                      )}
                    >
                      {h.host_display_name}
                    </button>
                  ))}
                </div>
              </div>
            </div>
            <DialogFooter>
              <Button type="button" variant="secondary" onClick={() => setDialogOpen(false)}>
                取消
              </Button>
              <Button type="submit">{editingId ? '保存' : '创建'}</Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>
    </div>
  )
}

function ThreadBucketColumn({
  title,
  bucket,
  tone,
  sessions,
  empty,
  onOpen,
}: {
  title: string
  bucket: ThreadBucket
  tone: 'primary' | 'success' | 'destructive'
  sessions: SessionSummary[]
  empty: string
  onOpen: (t: SessionSummary) => void
}) {
  const icon =
    bucket === 'running' ? <Activity size={14} /> : bucket === 'done' ? <CheckCircle2 size={14} /> : <XCircle size={14} />
  const toneClass =
    tone === 'primary'
      ? 'text-primary'
      : tone === 'success'
      ? 'text-success'
      : 'text-destructive'

  return (
    <div className="flex flex-col gap-3 rounded-xl border border-border/70 bg-background/40 p-4">
      <div className="flex items-center justify-between">
        <div className={cn('flex items-center gap-2 text-sm font-semibold', toneClass)}>
          {icon}
          <span>{title}</span>
        </div>
        <Badge variant="outline" className="mono">
          {sessions.length}
        </Badge>
      </div>
      <div className="flex flex-col gap-2">
        {sessions.length === 0 ? (
          <p className="rounded-lg border border-dashed border-border/70 px-3 py-6 text-center text-xs text-muted-foreground">
            {empty}
          </p>
        ) : (
          sessions.slice(0, 10).map((t) => (
            <motion.button
              key={t.session_id}
              type="button"
              onClick={() => onOpen(t)}
              whileHover={{ y: -1 }}
              className="flex flex-col gap-1 rounded-lg border border-border/60 bg-card px-3 py-2.5 text-left transition-all hover:border-primary/40"
            >
              <div className="flex items-center gap-2">
                <Hash size={12} className={cn('shrink-0', toneClass)} />
                <span className="truncate text-sm font-medium">
                  {t.title?.trim() || '未命名会话'}
                </span>
              </div>
              <div className="flex items-center gap-2 pl-[18px] text-[11px] text-muted-foreground">
                <span className="mono">{t.agent}</span>
                <span>·</span>
                <span className="flex items-center gap-1">
                  <Clock3 size={10} />
                  {formatRelative(t.last_ts_ms)}
                </span>
              </div>
              {t.end_reason ? (
                <div className="pl-[18px] text-[11px] text-muted-foreground">
                  {t.end_reason.kind}
                </div>
              ) : null}
            </motion.button>
          ))
        )}
      </div>
    </div>
  )
}

function SummaryStat({
  icon,
  label,
  value,
  hint,
}: {
  icon: React.ReactNode
  label: string
  value: string
  hint?: string
}) {
  return (
    <div className="flex flex-col gap-1 rounded-lg border border-border/60 bg-card px-3 py-2.5">
      <div className="mono flex items-center gap-1.5 text-[10px] uppercase tracking-widest text-muted-foreground">
        {icon}
        {label}
      </div>
      <div className="text-sm font-medium">{value}</div>
      {hint ? <div className="mono line-clamp-1 text-[11px] text-muted-foreground">{hint}</div> : null}
    </div>
  )
}
