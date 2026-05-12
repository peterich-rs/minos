import { useEffect, useState } from 'react'
import { useTheme } from 'next-themes'
import {
  Laptop,
  Monitor,
  Moon,
  Palette,
  Server,
  Sun,
  Wifi,
  WifiOff,
} from 'lucide-react'

import { Badge } from '@/components/ui/badge'
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import { ScrollArea } from '@/components/ui/scroll-area'
import { backendHttpBase, backendWsBase } from '@/lib/minos'
import { useAppStore } from '@/lib/store'
import { cn } from '@/lib/utils'

type ThemeOption = 'light' | 'dark' | 'system'

const THEMES: Array<{ value: ThemeOption; label: string; icon: typeof Sun }> = [
  { value: 'light', label: '浅色', icon: Sun },
  { value: 'dark', label: '深色', icon: Moon },
  { value: 'system', label: '跟随系统', icon: Laptop },
]

export function SettingsWorkspace() {
  const { theme, setTheme } = useTheme()
  const [mounted, setMounted] = useState(false)
  useEffect(() => {
    // Avoid hydration mismatch; next-themes needs to resolve the theme client-side first.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setMounted(true)
  }, [])

  const { connectionState, deviceId, session } = useAppStore()

  const stateCopy =
    connectionState === 'connected'
      ? '已连接'
      : connectionState === 'connecting'
      ? '连接中'
      : connectionState === 'error'
      ? '连接失败'
      : '未连接'

  const stateTone: 'success' | 'warning' | 'destructive' | 'outline' =
    connectionState === 'connected'
      ? 'success'
      : connectionState === 'connecting'
      ? 'warning'
      : connectionState === 'error'
      ? 'destructive'
      : 'outline'

  return (
    <ScrollArea className="h-full">
      <div className="mx-auto flex max-w-4xl flex-col gap-4 p-6">
        <div>
          <h1 className="text-2xl font-bold tracking-tight">设置</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            全局偏好设置。只影响当前浏览器,不会同步到后端。
          </p>
        </div>

        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <Palette size={18} className="text-primary" />
              外观
            </CardTitle>
            <CardDescription>切换深色 / 浅色主题,或跟随操作系统。</CardDescription>
          </CardHeader>
          <CardContent>
            <div className="grid grid-cols-3 gap-3">
              {THEMES.map((t) => {
                const Icon = t.icon
                const active = mounted && theme === t.value
                return (
                  <button
                    key={t.value}
                    type="button"
                    onClick={() => setTheme(t.value)}
                    className={cn(
                      'flex flex-col items-center gap-2 rounded-xl border p-4 text-sm transition-colors',
                      active
                        ? 'border-primary/60 bg-primary/10 text-primary shadow-[0_8px_24px_-16px_hsl(var(--primary)/0.5)]'
                        : 'border-border hover:bg-muted/60',
                    )}
                  >
                    <Icon size={20} />
                    <span>{t.label}</span>
                  </button>
                )
              })}
            </div>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <Server size={18} className="text-primary" />
              连接状态
            </CardTitle>
            <CardDescription>
              查看 Relay 与后端的当前配置。所有变更在 .env.local 中生效。
            </CardDescription>
          </CardHeader>
          <CardContent className="flex flex-col gap-3">
            <InfoRow
              icon={
                connectionState === 'connected' ? (
                  <Wifi size={16} className="text-success" />
                ) : (
                  <WifiOff size={16} className="text-muted-foreground" />
                )
              }
              label="Relay 状态"
              value={
                <Badge variant={stateTone} className="mono">
                  {stateCopy}
                </Badge>
              }
            />
            <InfoRow
              icon={<Monitor size={16} className="text-muted-foreground" />}
              label="HTTP Base URL"
              value={<code className="mono text-xs">{backendHttpBase()}</code>}
            />
            <InfoRow
              icon={<Monitor size={16} className="text-muted-foreground" />}
              label="WebSocket Base URL"
              value={<code className="mono text-xs">{backendWsBase()}</code>}
            />
            <InfoRow
              icon={<Monitor size={16} className="text-muted-foreground" />}
              label="Device ID"
              value={<code className="mono text-xs">{deviceId.slice(0, 8)}…</code>}
            />
            {session ? (
              <InfoRow
                icon={<Monitor size={16} className="text-muted-foreground" />}
                label="Account ID"
                value={<code className="mono text-xs">{session.accountId.slice(0, 8)}…</code>}
              />
            ) : null}
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>关于</CardTitle>
            <CardDescription>Minos web console</CardDescription>
          </CardHeader>
          <CardContent className="flex flex-col gap-1 text-sm text-muted-foreground">
            <p>一个用来配对 Mac、发起 Agent 回合与管理社交关系的浏览器管理台。</p>
            <p className="mono text-xs text-muted-foreground/80">
              tech stack · React 19 · Vite · Tailwind · shadcn/ui · Framer Motion · Zustand
            </p>
          </CardContent>
        </Card>
      </div>
    </ScrollArea>
  )
}

function InfoRow({
  icon,
  label,
  value,
}: {
  icon: React.ReactNode
  label: string
  value: React.ReactNode
}) {
  return (
    <div className="flex items-center justify-between gap-2 rounded-lg border border-border/60 bg-background/40 px-4 py-2.5">
      <div className="flex items-center gap-2 text-sm text-muted-foreground">
        {icon}
        <span>{label}</span>
      </div>
      <div className="flex items-center gap-2 text-sm">{value}</div>
    </div>
  )
}
