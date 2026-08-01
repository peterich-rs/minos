import { useCallback, useEffect, useState } from 'react'
import { Link2, Monitor, RefreshCw } from 'lucide-react'
import { toast } from 'sonner'

import { cn } from '@/shared/lib/utils'
import { PageHeader } from '@/shared/ui/PageHeader'
import { listHosts, type HostSummary } from '@/lib/minos'
import { useAppStore } from '@/lib/store'

export function CloudHostsView() {
  const { deviceId, session, setSession, setHosts, setActiveHost, activeHost } =
    useAppStore()
  const [rows, setRows] = useState<HostSummary[]>([])
  const [loading, setLoading] = useState(false)

  const refresh = useCallback(async () => {
    if (!session) return
    setLoading(true)
    try {
      const response = await listHosts(deviceId, session.accessToken)
      setRows(response.hosts)
      setHosts(response.hosts)
      if (!activeHost && response.hosts[0]) {
        setActiveHost(response.hosts[0].host_device_id)
      }
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e))
    } finally {
      setLoading(false)
    }
  }, [activeHost, deviceId, session, setActiveHost, setHosts])

  useEffect(() => {
    void refresh()
  }, [refresh])

  return (
    <div className="flex h-full min-h-0 flex-col bg-canvas-soft/40">
      <PageHeader
        title="Hosts"
        description="Machines linked to this account via Desktop Host Link."
        badge={
          <span className="rounded-full bg-primary-soft px-2 py-0.5 text-2xs font-semibold tabular-nums text-primary-strong">
            {rows.length}
          </span>
        }
        action={
          <button
            type="button"
            onClick={() => void refresh()}
            disabled={loading || !session}
            className="inline-flex items-center gap-1.5 rounded-xl border border-ink/8 px-3 py-1.5 text-sm font-medium text-ink-muted hover:bg-surface"
          >
            <RefreshCw className={cn('h-3.5 w-3.5', loading && 'animate-spin')} />
            Refresh
          </button>
        }
      />
      <div className="scrollbar-thin flex-1 space-y-3 overflow-y-auto p-5 sm:p-6">
        {!session ? (
          <p className="pt-8 text-center text-sm text-ink-muted">Sign in to list linked hosts.</p>
        ) : rows.length === 0 ? (
          <p className="pt-8 text-center text-sm text-ink-muted">
            No linked Macs. On Desktop, sign in with the same account and choose{' '}
            <strong>Link this Mac</strong>.
          </p>
        ) : (
          rows.map((h) => (
            <div
              key={h.host_device_id}
              className="flex items-center gap-4 rounded-2xl border border-ink/6 bg-surface px-4 py-4 shadow-panel transition-shadow hover:shadow-sm"
            >
              <div className="flex h-11 w-11 items-center justify-center rounded-xl bg-primary-soft text-primary">
                <Monitor className="h-5 w-5" />
              </div>
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  <span className="text-sm font-semibold text-ink">
                    {h.host_display_name}
                  </span>
                  <span
                    className={cn(
                      'h-1.5 w-1.5 rounded-full',
                      h.online ? 'bg-status-success' : 'bg-ink-faint',
                    )}
                  />
                  <span className="text-2xs capitalize text-ink-muted">
                    {h.online ? 'online' : 'offline'}
                  </span>
                </div>
                <div className="mt-0.5 flex items-center gap-1.5 text-2xs text-ink-muted">
                  <Link2 className="h-3 w-3" />
                  {h.host_device_id}
                </div>
              </div>
              <button
                type="button"
                onClick={() => setActiveHost(h.host_device_id)}
                className={cn(
                  'rounded-xl border px-3 py-1.5 text-sm font-medium',
                  activeHost === h.host_device_id
                    ? 'border-primary bg-primary-soft text-primary-strong'
                    : 'border-ink/8 text-ink-muted',
                )}
              >
                {activeHost === h.host_device_id ? 'Active' : 'Use'}
              </button>
            </div>
          ))
        )}
      </div>
    </div>
  )
}
