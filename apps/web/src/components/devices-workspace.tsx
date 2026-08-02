import { useEffect, useState } from 'react'
import { motion } from 'framer-motion'
import { toast } from 'sonner'
import {
  CheckCircle2,
  Cpu,
  FolderTree,
  Laptop2,
  RefreshCw,
  Trash2,
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
import { ScrollArea } from '@/components/ui/scroll-area'
import { Switch } from '@/components/ui/switch'
import {
  type HostSkillSummary,
  type HostSkillsEntry,
  type HostSummary,
  type ListHostSkillsResponse,
  type WriteHostSkillConfigResponse,
  listHosts,
  runWithSessionRefresh,
} from '@/lib/minos'
import { useAppStore } from '@/lib/store'
import { cn } from '@/lib/utils'

function formatDate(ms: number): string {
  return new Intl.DateTimeFormat(undefined, {
    month: 'short',
    day: 'numeric',
    year: 'numeric',
  }).format(ms)
}

export function DevicesWorkspace() {
  const {
    deviceId,
    session,
    setSession,
    hosts,
    setHosts,
    activeHost,
    setActiveHost,
    connectionState,
    relaySocket,
  } = useAppStore()
  const activeSession = session!

  const [hostsRefreshing, setHostsRefreshing] = useState(false)
  const [skillsByHost, setSkillsByHost] = useState<Record<string, HostSkillsEntry[]>>({})
  const [skillsError, setSkillsError] = useState<Record<string, string>>({})
  const [skillsRefreshing, setSkillsRefreshing] = useState(false)

  const effectiveHost = activeHost ?? hosts[0]?.host_device_id ?? null
  const skills = effectiveHost ? skillsByHost[effectiveHost] ?? [] : []
  const hostError = effectiveHost ? skillsError[effectiveHost] ?? null : null
  const loadingSkills = Boolean(
    effectiveHost &&
      connectionState === 'connected' &&
      !skillsByHost[effectiveHost] &&
      !skillsError[effectiveHost],
  )

  async function refreshHosts() {
    setHostsRefreshing(true)
    try {
      const response = await runWithSessionRefresh(activeSession, deviceId, setSession, (current) =>
        listHosts(deviceId, current.accessToken),
      )
      setHosts(response.hosts)
      if (!activeHost && response.hosts[0]) {
        setActiveHost(response.hosts[0].host_device_id)
      }
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e))
    } finally {
      setHostsRefreshing(false)
    }
  }

  async function loadSkills(hostId: string, force: boolean) {
    if (!relaySocket || connectionState !== 'connected') return
    if (force) setSkillsRefreshing(true)
    try {
      const response = await relaySocket.sendRpc<ListHostSkillsResponse>(
        hostId,
        'minos_list_host_skills',
        { workspace: '', force_reload: force },
      )
      setSkillsByHost((prev) => ({ ...prev, [hostId]: response.data }))
      setSkillsError((prev) => {
        const next = { ...prev }
        delete next[hostId]
        return next
      })
    } catch (e) {
      setSkillsError((prev) => ({
        ...prev,
        [hostId]: e instanceof Error ? e.message : String(e),
      }))
    } finally {
      if (force) setSkillsRefreshing(false)
    }
  }

  async function toggleSkill(hostId: string, skill: HostSkillSummary, enabled: boolean) {
    if (!relaySocket) return
    try {
      await relaySocket.sendRpc<WriteHostSkillConfigResponse>(
        hostId,
        'minos_write_host_skill_config',
        { workspace: '', path: skill.path, enabled },
      )
      await loadSkills(hostId, true)
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e))
    }
  }

  useEffect(() => {
    if (!effectiveHost || connectionState !== 'connected') return
    if (skillsByHost[effectiveHost] || skillsError[effectiveHost]) return
    void loadSkills(effectiveHost, false)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [effectiveHost, connectionState])

  return (
    <div className="flex h-full flex-col gap-4 overflow-hidden p-4">
      {/* Hosts list */}
      <Card className="flex flex-col">
        <CardHeader className="flex-row items-center justify-between space-y-0 pb-3">
          <div>
            <CardTitle>已链接 Mac</CardTitle>
            <CardDescription>
              同账号在 Desktop 上 Link 的主机。可在此切换当前运行上下文。
            </CardDescription>
          </div>
          <div className="flex items-center gap-2">
            <Button
              variant="outline"
              size="sm"
              onClick={refreshHosts}
              disabled={hostsRefreshing}
            >
              <RefreshCw size={14} className={cn(hostsRefreshing && 'animate-spin')} />
              刷新
            </Button>
          </div>
        </CardHeader>
        <CardContent className="pt-0">
          {hosts.length === 0 ? (
            <div className="flex flex-col items-center justify-center gap-3 rounded-xl border border-dashed border-border/70 p-8 text-center">
              <div className="flex size-12 items-center justify-center rounded-2xl bg-primary/10 text-primary">
                <Laptop2 size={22} />
              </div>
              <div>
                <p className="text-sm font-medium">还没有链接的 Mac</p>
                <p className="mt-1 text-xs text-muted-foreground">
                  在 Desktop 登录同一账号后执行 “Link this Mac”。
                </p>
              </div>
            </div>
          ) : (
            <div className="grid grid-cols-1 gap-3 md:grid-cols-2 xl:grid-cols-3">
              {hosts.map((host) => (
                <HostCard
                  key={host.host_device_id}
                  host={host}
                  active={effectiveHost === host.host_device_id}
                  onSelect={() => setActiveHost(host.host_device_id)}
                />
              ))}
            </div>
          )}
        </CardContent>
      </Card>

      {/* Skills panel */}
      <Card className="flex flex-1 flex-col overflow-hidden">
        <CardHeader className="flex-row items-center justify-between space-y-0 pb-3">
          <div className="min-w-0">
            <CardTitle>主机技能</CardTitle>
            <CardDescription>
              扫描当前选中主机的 superpowers 目录,可快速启用或停用某个技能。
            </CardDescription>
          </div>
          <Button
            variant="outline"
            size="sm"
            disabled={!effectiveHost || connectionState !== 'connected' || skillsRefreshing}
            onClick={() => effectiveHost && loadSkills(effectiveHost, true)}
          >
            <RefreshCw size={14} className={cn(skillsRefreshing && 'animate-spin')} />
            重新扫描
          </Button>
        </CardHeader>
        <CardContent className="flex flex-1 flex-col overflow-hidden pt-0">
          {!effectiveHost ? (
            <SkillsEmpty icon={<Cpu size={18} />} title="尚未选择主机" copy="先在上方配对一个 Mac。" />
          ) : connectionState !== 'connected' ? (
            <SkillsEmpty icon={<RefreshCw size={18} />} title="连接未就绪" copy="等待 Relay 连接后再试。" />
          ) : loadingSkills ? (
            <SkillsEmpty
              icon={<RefreshCw size={18} className="animate-spin" />}
              title="扫描中"
              copy="正在读取主机技能清单…"
            />
          ) : hostError ? (
            <SkillsEmpty icon={<Trash2 size={18} />} title="扫描失败" copy={hostError} />
          ) : skills.length === 0 ? (
            <SkillsEmpty icon={<FolderTree size={18} />} title="未发现技能" copy="当前主机没有可用的 superpowers 技能。" />
          ) : (
            <ScrollArea className="flex-1 pr-2">
              <div className="flex flex-col gap-4">
                {skills.map((entry) => (
                  <section
                    key={entry.cwd}
                    className="rounded-xl border border-border/70 bg-background/40 p-3"
                  >
                    <div className="mb-2 flex items-center gap-2">
                      <Badge variant="outline" className="mono">
                        {entry.cwd || '(default)'}
                      </Badge>
                      {entry.errors.length > 0 ? (
                        <Badge variant="destructive">{entry.errors.length} 错误</Badge>
                      ) : null}
                    </div>
                    {entry.errors.map((err) => (
                      <div
                        key={`${entry.cwd}-${err.path}`}
                        className="mb-2 rounded-lg bg-destructive/8 px-3 py-2 text-xs text-destructive"
                      >
                        <strong>{err.message}</strong>
                        <p className="mono text-muted-foreground">{err.path}</p>
                      </div>
                    ))}
                    <div className="grid grid-cols-1 gap-2 lg:grid-cols-2">
                      {entry.skills.map((skill) => (
                        <div
                          key={`${entry.cwd}-${skill.path}`}
                          className="flex items-start justify-between gap-3 rounded-lg border border-border/60 bg-card p-3"
                        >
                          <div className="min-w-0 flex-1">
                            <div className="flex items-center gap-2">
                              <h4 className="truncate text-sm font-semibold">
                                {skill.display_name?.trim() || skill.name}
                              </h4>
                              <Badge variant="outline" className="text-[10px]">
                                {skill.scope}
                              </Badge>
                            </div>
                            <p className="mt-1 line-clamp-2 text-xs text-muted-foreground">
                              {skill.short_description?.trim() || skill.description}
                            </p>
                            <p className="mono mt-1 line-clamp-1 text-[10px] text-muted-foreground/80">
                              {skill.path}
                            </p>
                          </div>
                          <Switch
                            checked={skill.enabled}
                            onCheckedChange={(checked) =>
                              void toggleSkill(effectiveHost, skill, checked)
                            }
                          />
                        </div>
                      ))}
                    </div>
                  </section>
                ))}
              </div>
            </ScrollArea>
          )}
        </CardContent>
      </Card>
    </div>
  )
}

function HostCard({
  host,
  active,
  onSelect,
}: {
  host: HostSummary
  active: boolean
  onSelect: () => void
}) {
  return (
    <motion.button
      type="button"
      onClick={onSelect}
      whileHover={{ y: -2 }}
      className={cn(
        'group relative flex flex-col gap-3 rounded-xl border bg-background/40 p-4 text-left transition-all',
        active
          ? 'border-primary/50 bg-primary/5 shadow-[0_10px_30px_-15px_hsl(var(--primary)/0.45)]'
          : 'border-border/70 hover:border-primary/30 hover:bg-muted/40',
      )}
    >
      <div className="flex items-center justify-between gap-2">
        <div className="flex size-10 items-center justify-center rounded-xl bg-primary/10 text-primary">
          <Laptop2 size={20} />
        </div>
        {active ? (
          <Badge variant="success">
            <CheckCircle2 size={12} />
            当前
          </Badge>
        ) : null}
      </div>
      <div>
        <h4 className="truncate text-sm font-semibold">{host.host_display_name}</h4>
        <p className="mono line-clamp-1 text-[11px] text-muted-foreground">
          {host.host_device_id}
        </p>
      </div>
      <p className="text-[11px] text-muted-foreground">
        配对于 {formatDate(host.paired_at_ms)}
      </p>
    </motion.button>
  )
}

function SkillsEmpty({
  icon,
  title,
  copy,
}: {
  icon: React.ReactNode
  title: string
  copy: string
}) {
  return (
    <div className="flex flex-1 flex-col items-center justify-center gap-3 rounded-xl border border-dashed border-border/70 p-10 text-center">
      <div className="flex size-12 items-center justify-center rounded-2xl bg-muted text-muted-foreground">
        {icon}
      </div>
      <div>
        <p className="text-sm font-medium">{title}</p>
        <p className="mt-1 max-w-sm text-xs text-muted-foreground">{copy}</p>
      </div>
    </div>
  )
}
