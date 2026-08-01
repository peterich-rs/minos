import { Bot, Columns3, List } from 'lucide-react'

import { ComposerChrome } from '@/shared/ui/ComposerChrome'
import {
  MessageAvatarGutter,
  MessageBody,
  MessageChrome,
  MessageSystemChrome,
} from '@/shared/ui/MessageChrome'
import { Avatar } from '@/shared/ui/Avatar'
import {
  WorkConversationRail,
  WorkConversationRow,
  WorkProjectHeader,
  WorkSurface,
  WorkTimelineHeader,
  WorkTimelineShell,
} from '@/shared/ui/WorkChrome'
import { cn } from '@/shared/lib/utils'

import { useAppStore } from '@/lib/store'

import {
  MOCK_MESSAGES,
  MOCK_PROJECTS,
  MOCK_SESSIONS,
  statusDotClass,
} from './mock-data'

/**
 * Web Work view — same WorkChrome / MessageChrome / ComposerChrome as Desktop.
 * Data is mock until CloudPort wires real hosts/sessions.
 */
export function CloudWorkView() {
  const projectId = useAppStore((s) => s.mockProjectId)
  const sessionId = useAppStore((s) => s.mockSessionId)
  const setSessionId = useAppStore((s) => s.setMockSessionId)

  const project =
    MOCK_PROJECTS.find((p) => p.id === projectId) ?? MOCK_PROJECTS[0]
  const sessions = MOCK_SESSIONS.filter((s) => s.projectId === project.id)
  const activeSession =
    sessions.find((s) => s.id === sessionId) ?? sessions[0] ?? null

  return (
    <WorkSurface>
      <WorkProjectHeader
        projectName={project.name}
        projectPath={project.path}
        tabs={[
          {
            id: 'conversations',
            label: 'Conversations',
            icon: <List className="h-3.5 w-3.5" />,
          },
          {
            id: 'sessions',
            label: 'Sessions',
            icon: <Bot className="h-3.5 w-3.5" />,
          },
          {
            id: 'board',
            label: 'Board',
            icon: <Columns3 className="h-3.5 w-3.5" />,
          },
        ]}
        activeTabId="conversations"
        searchDisabled
        newDisabled
        meta={
          <span className="tabular-nums">
            <strong className="font-semibold text-ink-secondary">
              {sessions.length}
            </strong>{' '}
            conversations
          </span>
        }
      />

      <div className="flex min-h-0 min-w-0 flex-1 overflow-hidden">
        <WorkConversationRail subtitle={`${sessions.length} in project`}>
          <div className="scrollbar-thin min-h-0 flex-1 space-y-0.5 overflow-y-auto">
            {sessions.map((s) => (
              <div key={s.id} className="pb-0.5">
                <WorkConversationRow
                  title={s.title}
                  preview={s.preview}
                  selected={activeSession?.id === s.id}
                  onSelect={() => setSessionId(s.id)}
                  titleTrailing={
                    <span className="shrink-0 text-2xs tabular-nums text-ink-muted">
                      {s.updatedLabel}
                    </span>
                  }
                  meta={
                    <>
                      <span className="inline-flex items-center gap-1 text-2xs text-ink-muted">
                        <Bot className="h-3 w-3" />
                        {s.agent}
                      </span>
                      <span
                        className={cn(
                          'ml-auto h-1.5 w-1.5 rounded-full',
                          statusDotClass(s.status),
                        )}
                      />
                    </>
                  }
                />
              </div>
            ))}
          </div>
        </WorkConversationRail>

        <WorkTimelineShell
          header={
            <WorkTimelineHeader
              title={activeSession?.title ?? 'No conversation'}
              subtitle={
                activeSession
                  ? `${activeSession.agent} · ${activeSession.status}`
                  : 'Select a conversation'
              }
            />
          }
          composer={
            <ComposerChrome
              textareaProps={{
                disabled: true,
                rows: 3,
                placeholder:
                  'Message… type @ to mention an agent (e.g. @grok hello)',
              }}
              sendDisabled
              sendLabel="Send"
              hint="Cloud · @member agent · ⌘/Ctrl+Enter send · CloudPort next"
            />
          }
        >
          <div className="scrollbar-thin h-full space-y-0.5 overflow-y-auto px-3 py-4 sm:px-5">
            {MOCK_MESSAGES.map((m, i) => {
              const cont =
                i > 0 &&
                MOCK_MESSAGES[i - 1]?.role === m.role &&
                m.role !== 'system'
              if (m.role === 'system') {
                return (
                  <MessageSystemChrome key={m.id}>{m.body}</MessageSystemChrome>
                )
              }
              const isUser = m.role === 'user'
              const authorLabel = isUser ? 'You' : 'Assistant'
              return (
                <MessageChrome
                  key={m.id}
                  messageId={m.id}
                  groupedWithPrevious={cont}
                  avatar={
                    cont ? (
                      <div aria-hidden className="w-9 shrink-0" />
                    ) : (
                      <MessageAvatarGutter>
                        <Avatar
                          name={authorLabel}
                          tone={isUser ? 'slate' : 'purple'}
                          size="md"
                        />
                      </MessageAvatarGutter>
                    )
                  }
                  header={
                    cont ? null : (
                      <div className="flex min-w-0 flex-wrap items-baseline gap-x-1.5 gap-y-0 leading-4">
                        <span className="truncate text-sm font-semibold leading-4 tracking-tight text-ink">
                          {authorLabel}
                        </span>
                      </div>
                    )
                  }
                >
                  <MessageBody grouped={cont}>{m.body}</MessageBody>
                </MessageChrome>
              )
            })}
          </div>
        </WorkTimelineShell>
      </div>
    </WorkSurface>
  )
}
