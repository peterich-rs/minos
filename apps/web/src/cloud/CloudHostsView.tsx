import { Link2, Monitor } from 'lucide-react'

import { cn } from '@/shared/lib/utils'
import { PageHeader } from '@/shared/ui/PageHeader'

import { MOCK_HOSTS, statusDotClass } from './mock-data'

export function CloudHostsView() {
  return (
    <div className="flex h-full min-h-0 flex-col bg-canvas-soft/40">
      <PageHeader
        title="Hosts"
        description="Machines linked to this account. Mock until Host Link API ships."
        badge={
          <span className="rounded-full bg-primary-soft px-2 py-0.5 text-2xs font-semibold tabular-nums text-primary-strong">
            {MOCK_HOSTS.length}
          </span>
        }
      />
      <div className="scrollbar-thin flex-1 space-y-3 overflow-y-auto p-5 sm:p-6">
        {MOCK_HOSTS.map((h) => (
          <div
            key={h.id}
            className="flex items-center gap-4 rounded-2xl border border-ink/6 bg-surface px-4 py-4 shadow-panel transition-shadow hover:shadow-sm"
          >
            <div className="flex h-11 w-11 items-center justify-center rounded-xl bg-primary-soft text-primary">
              <Monitor className="h-5 w-5" />
            </div>
            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-2">
                <span className="text-sm font-semibold text-ink">{h.name}</span>
                <span
                  className={cn(
                    'h-1.5 w-1.5 rounded-full',
                    statusDotClass(h.status),
                  )}
                />
                <span className="text-2xs capitalize text-ink-muted">
                  {h.status}
                </span>
              </div>
              <div className="mt-0.5 flex items-center gap-1.5 text-2xs text-ink-muted">
                <Link2 className="h-3 w-3" />
                {h.linked ? 'Linked to account' : 'Not linked'}
              </div>
            </div>
            <button
              type="button"
              disabled
              className="rounded-xl border border-ink/8 px-3 py-1.5 text-sm font-medium text-ink-muted"
            >
              {h.linked ? 'Unlink' : 'Link'}
            </button>
          </div>
        ))}
        <p className="pt-2 text-center text-2xs text-ink-faint">
          Real list: GET /v1/hosts after Host Link (D02)
        </p>
      </div>
    </div>
  )
}
