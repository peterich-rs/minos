import { AlertTriangle } from 'lucide-react'

import {
  AttentionListCard,
  AttentionPrimaryButton,
} from '@/shared/ui/AttentionChrome'
import { PageHeader } from '@/shared/ui/PageHeader'
import { cn } from '@/shared/lib/utils'

import { MOCK_PROJECTS, MOCK_SESSIONS } from './mock-data'

/**
 * Web Attention — same AttentionChrome cards as Desktop AttentionView.
 */
export function CloudAttentionView() {
  const items = MOCK_SESSIONS.filter(
    (s) => s.status === 'needs_approval' || s.status === 'failed',
  )
  const attentionProjects = MOCK_PROJECTS.filter((p) => p.needsAttention > 0)

  return (
    <div className="flex h-full min-h-0 flex-col bg-canvas-soft/40">
      <PageHeader
        title={
          <span className="inline-flex items-center gap-2">
            <AlertTriangle className="h-6 w-6 text-status-approval" />
            Attention
          </span>
        }
        description="Approvals and failures across linked hosts (mock)."
        badge={
          items.length > 0 ? (
            <span className="rounded-full bg-status-approval/15 px-2 py-0.5 text-2xs font-semibold tabular-nums text-status-approval">
              {items.length}
            </span>
          ) : null
        }
      />
      <div className="scrollbar-thin flex-1 space-y-3 overflow-y-auto p-5 sm:p-6">
        {attentionProjects.map((p) => (
          <AttentionListCard
            key={p.id}
            tone="neutral"
            title={p.name}
            body={`${p.needsAttention} item(s) need attention`}
            meta={p.lastActiveLabel}
          />
        ))}
        {items.map((s) => {
          const isApproval = s.status === 'needs_approval'
          return (
            <AttentionListCard
              key={s.id}
              tone={isApproval ? 'approval' : 'failed'}
              title={isApproval ? 'Approval required' : 'Session failed'}
              badge={
                <span
                  className={cn(
                    'rounded-md px-1.5 py-0.5 text-2xs font-medium',
                    'bg-ink/10 text-ink-secondary',
                  )}
                >
                  {s.agent}
                </span>
              }
              body={s.preview}
              meta={`${s.updatedLabel} · ${s.status}`}
              actions={
                <AttentionPrimaryButton disabled>
                  {isApproval ? 'Review / approve' : 'Open transcript'}
                </AttentionPrimaryButton>
              }
            />
          )
        })}
        {items.length === 0 ? (
          <p className="py-12 text-center text-sm text-ink-muted">
            Nothing needs attention (mock empty).
          </p>
        ) : null}
      </div>
    </div>
  )
}
