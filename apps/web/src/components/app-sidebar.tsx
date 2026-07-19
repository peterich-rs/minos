import { useEffect, useState } from 'react'
import { motion } from 'framer-motion'
import { useTheme } from 'next-themes'
import {
  Bot,
  Cpu,
  ListChecks,
  MessageCircle,
  Moon,
  Settings,
  Sparkles,
  Sun,
  UserCircle2,
  Users,
  type LucideIcon,
} from 'lucide-react'

import { Button } from '@/components/ui/button'
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@/components/ui/tooltip'
import { useAppStore, type RouteKey } from '@/lib/store'
import { cn } from '@/lib/utils'

type NavItem = {
  key: RouteKey
  label: string
  icon: LucideIcon
  hint?: string
}

const MAIN_NAV: NavItem[] = [
  { key: 'chat', label: '聊天', icon: MessageCircle, hint: '与你的 AI 会话' },
  { key: 'tasks', label: '任务', icon: ListChecks, hint: '运行中的 Agent 任务与预设' },
  { key: 'friends', label: '伙伴', icon: Users, hint: '好友、群组与请求' },
  { key: 'devices', label: '设备', icon: Cpu, hint: '已配对 Mac 与主机技能' },
]

const FOOTER_NAV: NavItem[] = [
  { key: 'profile', label: '个人', icon: UserCircle2, hint: '账户与安全' },
  { key: 'settings', label: '设置', icon: Settings, hint: '主题与偏好' },
]

function ThemeToggle() {
  const [mounted, setMounted] = useState(false)
  const { resolvedTheme, setTheme } = useTheme()

  useEffect(() => {
    // Hydration guard — theme is only known client-side.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setMounted(true)
  }, [])
  const isDark = mounted && resolvedTheme === 'dark'

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          variant="ghost"
          size="icon"
          className="size-10 rounded-xl text-muted-foreground hover:text-foreground"
          onClick={() => setTheme(isDark ? 'light' : 'dark')}
          aria-label="切换主题"
        >
          {isDark ? <Sun size={18} /> : <Moon size={18} />}
        </Button>
      </TooltipTrigger>
      <TooltipContent side="right">切换{isDark ? '浅色' : '深色'}主题</TooltipContent>
    </Tooltip>
  )
}

function NavButton({
  item,
  active,
  onClick,
}: {
  item: NavItem
  active: boolean
  onClick: () => void
}) {
  const Icon = item.icon
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          type="button"
          onClick={onClick}
          aria-current={active ? 'page' : undefined}
          className={cn(
            'group relative flex size-12 items-center justify-center rounded-xl text-muted-foreground transition-colors',
            'hover:bg-sidebar-accent hover:text-sidebar-accent-foreground',
            'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-sidebar',
            active && 'text-sidebar-primary-foreground',
          )}
        >
          {active ? (
            <motion.span
              layoutId="sidebar-active"
              className="absolute inset-0 rounded-xl bg-sidebar-primary shadow-[0_8px_30px_-8px_hsl(var(--sidebar-primary)/0.6)]"
              transition={{ type: 'spring', stiffness: 420, damping: 32 }}
            />
          ) : null}
          <Icon size={20} strokeWidth={active ? 2.2 : 1.9} className="relative z-10" />
        </button>
      </TooltipTrigger>
      <TooltipContent side="right" className="flex flex-col gap-0.5">
        <span className="font-semibold">{item.label}</span>
        {item.hint ? (
          <span className="text-[11px] text-muted-foreground">{item.hint}</span>
        ) : null}
      </TooltipContent>
    </Tooltip>
  )
}

export function AppSidebar() {
  const { route, setRoute, connectionState } = useAppStore()

  const connectionDot =
    connectionState === 'connected'
      ? 'bg-success'
      : connectionState === 'connecting'
      ? 'bg-warning animate-pulse'
      : connectionState === 'error'
      ? 'bg-destructive'
      : 'bg-muted-foreground/40'

  return (
    <TooltipProvider delayDuration={120}>
      <aside className="relative z-20 flex h-full w-[72px] shrink-0 flex-col items-center gap-3 border-r border-sidebar-border bg-sidebar py-4">
        <div className="relative flex size-11 items-center justify-center rounded-2xl bg-gradient-to-br from-primary via-primary to-[hsl(var(--primary)/0.7)] text-primary-foreground shadow-lg">
          <Sparkles size={18} />
          <span
            className={cn(
              'absolute -bottom-0.5 -right-0.5 size-2.5 rounded-full ring-2 ring-sidebar',
              connectionDot,
            )}
            aria-hidden
          />
        </div>

        <div className="mt-4 flex flex-1 flex-col items-center gap-2">
          {MAIN_NAV.map((item) => (
            <NavButton
              key={item.key}
              item={item}
              active={route === item.key}
              onClick={() => setRoute(item.key)}
            />
          ))}
        </div>

        <div className="flex flex-col items-center gap-2 pb-1">
          <ThemeToggle />
          {FOOTER_NAV.map((item) => (
            <NavButton
              key={item.key}
              item={item}
              active={route === item.key}
              onClick={() => setRoute(item.key)}
            />
          ))}
        </div>

        <span className="mono text-[9px] font-medium uppercase tracking-[0.2em] text-muted-foreground/70">
          <Bot size={12} className="mx-auto mb-0.5" />
          v1
        </span>
      </aside>
    </TooltipProvider>
  )
}
