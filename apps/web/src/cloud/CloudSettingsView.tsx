import { PageHeader } from '@/shared/ui/PageHeader'
import { useAppStore } from '@/lib/store'
import { backendHttpBase } from '@/lib/minos'
import { isSupabaseConfigured } from '@/lib/supabase'

export function CloudSettingsView() {
  const session = useAppStore((s) => s.session)
  const deviceId = useAppStore((s) => s.deviceId)

  return (
    <div className="flex h-full min-h-0 flex-col bg-canvas-soft/40">
      <PageHeader
        title="Settings"
        description="Account and origin. Desktop settings parity later."
      />
      <div className="scrollbar-thin flex-1 space-y-4 overflow-y-auto p-5 sm:p-6">
        <section className="rounded-2xl border border-ink/6 bg-surface p-5 shadow-panel">
          <h2 className="text-lg font-semibold tracking-tight text-ink">
            Account
          </h2>
          <dl className="mt-4 space-y-3 text-sm">
            <div className="flex justify-between gap-4 border-b border-ink/5 pb-3">
              <dt className="text-ink-muted">Email</dt>
              <dd className="truncate font-medium text-ink">
                {session?.email ?? '—'}
              </dd>
            </div>
            <div className="flex justify-between gap-4 border-b border-ink/5 pb-3">
              <dt className="text-ink-muted">Account id</dt>
              <dd className="truncate font-mono text-2xs text-ink">
                {session?.accountId ?? '—'}
              </dd>
            </div>
            <div className="flex justify-between gap-4">
              <dt className="text-ink-muted">Device id</dt>
              <dd className="truncate font-mono text-2xs text-ink">{deviceId}</dd>
            </div>
          </dl>
        </section>

        <section className="rounded-2xl border border-ink/6 bg-surface p-5 shadow-panel">
          <h2 className="text-lg font-semibold tracking-tight text-ink">
            Origins
          </h2>
          <dl className="mt-4 space-y-3 text-sm">
            <div className="flex justify-between gap-4 border-b border-ink/5 pb-3">
              <dt className="text-ink-muted">Minos backend</dt>
              <dd className="truncate font-mono text-2xs text-ink">
                {backendHttpBase()}
              </dd>
            </div>
            <div className="flex justify-between gap-4">
              <dt className="text-ink-muted">Supabase IdP</dt>
              <dd className="font-medium text-ink">
                {isSupabaseConfigured() ? 'configured' : 'off'}
              </dd>
            </div>
          </dl>
        </section>

        <p className="text-center text-2xs text-ink-faint">
          PageHeader + cards share Desktop SSOT (
          <code className="font-mono">@/shared/ui/PageHeader</code>).
        </p>
      </div>
    </div>
  )
}
