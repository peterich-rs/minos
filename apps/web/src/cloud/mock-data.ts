/**
 * Mock product data for the Desktop-aligned Web shell (CloudPort stub).
 * Replace with real CloudPort / REST + WS as hosts and projection land.
 */

export type CloudNav = 'work' | 'attention' | 'hosts' | 'settings'

export interface MockProject {
  id: string
  name: string
  path: string
  needsAttention: number
  lastActiveLabel: string
}

export interface MockHost {
  id: string
  name: string
  status: 'online' | 'offline' | 'degraded'
  linked: boolean
}

export interface MockSession {
  id: string
  projectId: string
  title: string
  agent: string
  status: 'idle' | 'running' | 'needs_approval' | 'done' | 'failed'
  preview: string
  updatedLabel: string
}

export interface MockMessage {
  id: string
  role: 'user' | 'assistant' | 'system'
  body: string
}

export const MOCK_PROJECTS: MockProject[] = [
  {
    id: 'proj-minos',
    name: 'Minos',
    path: '~/code/github.com/Minos',
    needsAttention: 2,
    lastActiveLabel: '12m ago',
  },
  {
    id: 'proj-ainexc',
    name: 'ainexc-site',
    path: '~/code/ainexc',
    needsAttention: 0,
    lastActiveLabel: '2h ago',
  },
  {
    id: 'proj-notes',
    name: 'notes',
    path: '~/notes',
    needsAttention: 1,
    lastActiveLabel: 'Yesterday',
  },
]

export const MOCK_HOSTS: MockHost[] = [
  {
    id: 'host-mba',
    name: "Fan's MacBook Air",
    status: 'online',
    linked: true,
  },
  {
    id: 'host-studio',
    name: 'Studio Mini',
    status: 'offline',
    linked: false,
  },
]

export const MOCK_SESSIONS: MockSession[] = [
  {
    id: 'sess-1',
    projectId: 'proj-minos',
    title: 'Supabase auth exchange',
    agent: 'grok',
    status: 'done',
    preview: 'POST /v1/auth/supabase is green on Web…',
    updatedLabel: '8m ago',
  },
  {
    id: 'sess-2',
    projectId: 'proj-minos',
    title: 'Web UI port to Desktop chrome',
    agent: 'codex',
    status: 'running',
    preview: 'Scaffolding CloudShell + mock Work view…',
    updatedLabel: 'now',
  },
  {
    id: 'sess-3',
    projectId: 'proj-ainexc',
    title: 'Landing copy pass',
    agent: 'claude',
    status: 'needs_approval',
    preview: 'Wants to edit apps/web/index.html',
    updatedLabel: '1h ago',
  },
]

export const MOCK_MESSAGES: MockMessage[] = [
  {
    id: 'm1',
    role: 'user',
    body: '把 Web 控制台做成和 Desktop 一样的壳，功能可以先 mock。',
  },
  {
    id: 'm2',
    role: 'assistant',
    body: '可以。第一阶段对齐 Desktop 的 surface / ink token 和侧栏导航，Work 区用 mock 项目与会话列表；CloudPort 接真数据放在下一阶段。',
  },
  {
    id: 'm3',
    role: 'assistant',
    body: 'Auth 已走 Supabase → Minos exchange；登录后进入这套 mock shell。',
  },
]

export function statusDotClass(
  status: MockSession['status'] | MockHost['status'],
): string {
  switch (status) {
    case 'running':
    case 'online':
      return 'bg-status-running'
    case 'needs_approval':
      return 'bg-status-approval'
    case 'failed':
    case 'offline':
      return 'bg-status-failed'
    case 'done':
      return 'bg-status-done'
    case 'degraded':
      return 'bg-status-suspended'
    default:
      return 'bg-status-idle'
  }
}
