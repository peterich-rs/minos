import {
  AlertTriangle,
  Cloud,
  FolderGit2,
  LayoutDashboard,
  Monitor,
  Settings,
} from 'lucide-react'

import {
  AppRail,
  AppRailAccountFooter,
  AppRailProjectRow,
  AppRailProjectsHeader,
} from '@/shared/ui/AppRail'
import { cn } from '@/shared/lib/utils'

import { useAppStore } from '@/lib/store'
import { signOutSupabase } from '@/lib/supabase'

import { MOCK_PROJECTS } from './mock-data'

const navItems = [
  { id: 'work', label: 'Work', icon: LayoutDashboard },
  { id: 'attention', label: 'Attention', icon: AlertTriangle },
  { id: 'hosts', label: 'Hosts', icon: Monitor },
  { id: 'settings', label: 'Settings', icon: Settings },
] as const

export function CloudSidebar() {
  const primaryNav = useAppStore((s) => s.primaryNav)
  const setPrimaryNav = useAppStore((s) => s.setPrimaryNav)
  const projectId = useAppStore((s) => s.mockProjectId)
  const selectProject = useAppStore((s) => s.setMockProjectId)
  const session = useAppStore((s) => s.session)
  const logout = useAppStore((s) => s.logout)

  const attention = MOCK_PROJECTS.reduce((n, p) => n + p.needsAttention, 0)

  async function handleLogout() {
    try {
      await signOutSupabase()
    } catch {
      // best-effort
    }
    logout()
  }

  return (
    <AppRail
      brandSubtitle={
        <span className="flex items-center gap-1">
          <Cloud className="h-3 w-3" />
          Cloud
        </span>
      }
      navItems={navItems.map((item) => {
        const Icon = item.icon
        return {
          id: item.id,
          label: item.label,
          badge: item.id === 'attention' ? attention : 0,
          icon: <Icon strokeWidth={1.8} />,
        }
      })}
      activeNavId={primaryNav}
      onNavSelect={(id) =>
        setPrimaryNav(id as typeof primaryNav)
      }
      projectsHeader={
        <AppRailProjectsHeader
          action={<span className="text-3xs text-ink-faint">mock</span>}
        />
      }
      projects={
        <>
          {MOCK_PROJECTS.map((p) => {
            const active = projectId === p.id
            return (
              <AppRailProjectRow
                key={p.id}
                name={p.name}
                path={p.path}
                active={active}
                attention={p.needsAttention}
                onClick={() => {
                  selectProject(p.id)
                  setPrimaryNav('work')
                }}
                leading={
                  <FolderGit2
                    className={cn(
                      'mt-0.5 h-4 w-4 shrink-0',
                      active ? 'text-primary' : 'text-ink-muted',
                    )}
                    strokeWidth={1.8}
                  />
                }
              />
            )
          })}
        </>
      }
      footer={
        <AppRailAccountFooter
          email={session?.email ?? '—'}
          statusLabel="Linked · cloud"
          onSignOut={() => void handleLogout()}
        />
      }
    />
  )
}
