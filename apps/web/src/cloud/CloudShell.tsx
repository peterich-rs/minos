import { Toaster } from 'sonner'

import { ShellFrame } from '@/shared/layout/ShellFrame'
import { useAppStore } from '@/lib/store'

import { CloudAttentionView } from './CloudAttentionView'
import { CloudHostsView } from './CloudHostsView'
import { CloudSettingsView } from './CloudSettingsView'
import { CloudSidebar } from './CloudSidebar'
import { CloudWorkView } from './CloudWorkView'

/**
 * Web cloud console shell — same ShellFrame chrome as Desktop Host Console.
 * Visual SSOT: apps/desktop/src/shared/{layout/ShellFrame,styles/design-system}.
 * Data: mock until CloudPort.
 */
export function CloudShell() {
  const primaryNav = useAppStore((s) => s.primaryNav)

  return (
    <ShellFrame sidebar={<CloudSidebar />}>
      <main className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
        {primaryNav === 'work' ? <CloudWorkView /> : null}
        {primaryNav === 'attention' ? <CloudAttentionView /> : null}
        {primaryNav === 'hosts' ? <CloudHostsView /> : null}
        {primaryNav === 'settings' ? <CloudSettingsView /> : null}
      </main>
      <Toaster position="bottom-right" richColors closeButton />
    </ShellFrame>
  )
}
