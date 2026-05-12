import { useEffect, useRef, useState } from 'react'
import { motion, AnimatePresence } from 'framer-motion'
import {
  Bot,
  Hash,
  MessageSquarePlus,
  Search,
  Send,
  Sparkles,
  StopCircle,
  Zap,
} from 'lucide-react'

import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Textarea } from '@/components/ui/textarea'
import { useAppStore } from '@/lib/store'
import { transcriptFromEvents } from '@/lib/chat-utils'
import { cn } from '@/lib/utils'

import { ChatTranscript } from './chat-transcript'

function formatThreadTimestamp(ms: number): string {
  const delta = Date.now() - ms
  const min = Math.round(delta / 60_000)
  if (min < 1) return '刚刚'
  if (min < 60) return `${min} 分钟前`
  const hours = Math.round(min / 60)
  if (hours < 24) return `${hours} 小时前`
  const days = Math.round(hours / 24)
  if (days < 7) return `${days} 天前`
  return new Date(ms).toLocaleDateString()
}

export function ChatWorkspace() {
  const {
    threads,
    selectedThreadId,
    setSelectedThreadId,
    threadRecords,
    composerText,
    setComposerText,
    activeHost,
    relaySocket,
    connectionState,
  } = useAppStore()

  const [isSubmitting, setIsSubmitting] = useState(false)
  const [filter, setFilter] = useState('')
  const scrollRef = useRef<HTMLDivElement>(null)

  const selectedThread = threads.find((t) => t.thread_id === selectedThreadId) ?? null
  const currentRecord = selectedThreadId ? threadRecords[selectedThreadId] : null
  const transcript = transcriptFromEvents(currentRecord?.ui_events ?? [])

  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight
    }
  }, [transcript.messages.length, currentRecord?.ui_events.length])

  const filteredThreads = filter.trim()
    ? threads.filter((t) =>
        (t.title ?? 'Untitled').toLowerCase().includes(filter.trim().toLowerCase()),
      )
    : threads

  async function handleSend() {
    if (!composerText.trim() || !activeHost || !relaySocket || isSubmitting) return
    setIsSubmitting(true)
    try {
      if (!selectedThread) {
        await relaySocket.sendRpc(activeHost, 'minos_start_agent', {
          cwd: '',
          agent: 'codex',
          prompt: composerText.trim(),
        })
      } else {
        await relaySocket.sendRpc(activeHost, 'minos_send_user_message', {
          session_id: selectedThread.thread_id,
          text: composerText.trim(),
        })
      }
      setComposerText('')
    } catch (error) {
      console.error('发送失败', error)
    } finally {
      setIsSubmitting(false)
    }
  }

  async function handleInterrupt() {
    if (!selectedThread || !activeHost || !relaySocket) return
    try {
      await relaySocket.sendRpc(activeHost, 'minos_interrupt_thread', {
        thread_id: selectedThread.thread_id,
      })
    } catch (error) {
      console.error('中断失败', error)
    }
  }

  const canInteract = activeHost && connectionState === 'connected'

  return (
    <div className="flex h-full gap-4 p-4">
      {/* Left — thread list */}
      <aside className="flex w-72 shrink-0 flex-col overflow-hidden rounded-2xl border border-border/70 bg-card shadow-sm">
        <div className="flex items-center justify-between border-b border-border/60 px-4 py-3">
          <div>
            <p className="mono text-[10px] uppercase tracking-[0.18em] text-muted-foreground">
              Threads
            </p>
            <h3 className="text-sm font-semibold">会话记录</h3>
          </div>
          <Button
            size="icon"
            variant="ghost"
            className="size-8 rounded-lg"
            onClick={() => setSelectedThreadId(null)}
            title="新会话"
          >
            <MessageSquarePlus size={16} />
          </Button>
        </div>

        <div className="px-3 pt-3">
          <div className="relative">
            <Search
              size={14}
              className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-muted-foreground"
            />
            <Input
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
              placeholder="搜索会话"
              className="h-9 pl-8"
            />
          </div>
        </div>

        <ScrollArea className="flex-1 px-2 py-3">
          <button
            type="button"
            onClick={() => setSelectedThreadId(null)}
            className={cn(
              'group mb-1 flex w-full items-center gap-2 rounded-xl px-3 py-2.5 text-left transition-colors',
              !selectedThreadId
                ? 'bg-primary/10 text-primary'
                : 'text-muted-foreground hover:bg-muted/70 hover:text-foreground',
            )}
          >
            <Sparkles size={16} className="shrink-0" />
            <div className="min-w-0 flex-1">
              <div className="truncate text-sm font-medium">开启新会话</div>
              <div className="mono text-[11px] text-muted-foreground/80">
                空白上下文
              </div>
            </div>
          </button>

          {filteredThreads.length === 0 ? (
            <div className="mt-6 px-4 text-center">
              <p className="text-xs text-muted-foreground">
                {filter ? '没有匹配的会话' : '暂无历史会话'}
              </p>
            </div>
          ) : (
            filteredThreads.map((thread) => {
              const active = selectedThreadId === thread.thread_id
              const ended = thread.ended_at_ms != null
              return (
                <button
                  key={thread.thread_id}
                  type="button"
                  onClick={() => setSelectedThreadId(thread.thread_id)}
                  className={cn(
                    'mb-1 flex w-full flex-col gap-1 rounded-xl px-3 py-2.5 text-left transition-colors',
                    active
                      ? 'bg-primary/10 text-foreground ring-1 ring-inset ring-primary/30'
                      : 'hover:bg-muted/70',
                  )}
                >
                  <div className="flex items-center gap-2">
                    <Hash
                      size={14}
                      className={cn(
                        'shrink-0',
                        active ? 'text-primary' : 'text-muted-foreground',
                      )}
                    />
                    <span className="truncate text-sm font-medium">
                      {thread.title?.trim() || '未命名会话'}
                    </span>
                  </div>
                  <div className="flex items-center gap-2 pl-5 text-[11px] text-muted-foreground">
                    <span className="mono">{thread.agent}</span>
                    <span>·</span>
                    <span>{formatThreadTimestamp(thread.last_ts_ms)}</span>
                    {ended ? (
                      <Badge variant="outline" className="ml-auto shrink-0 h-4 px-1.5 text-[10px]">
                        已结束
                      </Badge>
                    ) : null}
                  </div>
                </button>
              )
            })
          )}
        </ScrollArea>
      </aside>

      {/* Right — conversation surface */}
      <main className="relative flex flex-1 flex-col overflow-hidden rounded-2xl border border-border/70 bg-card shadow-sm">
        <div className="flex items-center justify-between border-b border-border/60 px-6 py-4">
          <div className="min-w-0">
            <p className="mono text-[10px] uppercase tracking-[0.18em] text-muted-foreground">
              {selectedThread ? 'active thread' : 'new conversation'}
            </p>
            <h2 className="truncate text-base font-semibold">
              {selectedThread?.title?.trim() || '你想做点什么?'}
            </h2>
          </div>
          <div className="flex items-center gap-2">
            {selectedThread ? (
              <Badge variant={selectedThread.ended_at_ms ? 'outline' : 'success'}>
                {selectedThread.ended_at_ms ? '已结束' : '进行中'}
              </Badge>
            ) : null}
            {selectedThread ? (
              <Badge variant="outline" className="mono">
                {selectedThread.agent}
              </Badge>
            ) : null}
          </div>
        </div>

        <div className="flex-1 overflow-y-auto" ref={scrollRef}>
          <AnimatePresence mode="wait">
            {transcript.messages.length === 0 ? (
              <motion.div
                key="empty"
                initial={{ opacity: 0, y: 8 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0 }}
                className="flex h-full flex-col items-center justify-center p-10 text-center"
              >
                <div className="mb-6 flex size-16 items-center justify-center rounded-2xl bg-primary/10 text-primary shadow-inner">
                  <Bot size={30} />
                </div>
                <h2 className="mb-2 text-2xl font-bold tracking-tight">我可以帮你做点什么?</h2>
                <p className="max-w-md text-sm text-muted-foreground">
                  让 Minos 在已配对的设备上写代码、执行命令、编辑文件。回车发送,Shift + 回车换行。
                </p>
                <div className="mt-8 grid w-full max-w-lg grid-cols-1 gap-2 sm:grid-cols-2">
                  {[
                    '帮我分析这个仓库的目录结构',
                    '运行一下项目的测试套件',
                    '在 src 里创建一个新组件',
                    '解释最近一次提交的改动',
                  ].map((hint) => (
                    <button
                      key={hint}
                      type="button"
                      onClick={() => setComposerText(hint)}
                      className="rounded-xl border border-border/70 bg-background/40 px-3 py-2.5 text-left text-[13px] text-muted-foreground transition-colors hover:border-primary/40 hover:bg-accent hover:text-accent-foreground"
                    >
                      {hint}
                    </button>
                  ))}
                </div>
              </motion.div>
            ) : (
              <motion.div
                key="transcript"
                initial={{ opacity: 0 }}
                animate={{ opacity: 1 }}
                className="flex min-h-full flex-col justify-end"
              >
                <ChatTranscript messages={transcript.messages} />
              </motion.div>
            )}
          </AnimatePresence>
        </div>

        {/* Composer */}
        <div className="border-t border-border/60 bg-background/50 p-4 backdrop-blur-md">
          <div className="relative mx-auto flex max-w-4xl items-end gap-2 rounded-2xl border border-border bg-card p-2 shadow-sm transition-all focus-within:border-primary/50 focus-within:ring-4 focus-within:ring-primary/10">
            <Textarea
              value={composerText}
              onChange={(e) => setComposerText(e.target.value)}
              placeholder={
                canInteract ? '跟 Agent 说点什么…' : '尚未连接到任何 Mac 主机'
              }
              disabled={!canInteract}
              className="max-h-[240px] min-h-[44px] resize-none border-0 px-3 py-3 shadow-none focus-visible:ring-0"
              onKeyDown={(e) => {
                if (e.key === 'Enter' && !e.shiftKey) {
                  e.preventDefault()
                  handleSend()
                }
              }}
            />
            <div className="flex shrink-0 flex-col gap-2 p-1">
              {selectedThread && !selectedThread.ended_at_ms ? (
                <Button
                  size="icon"
                  variant="outline"
                  className="size-10 rounded-xl text-destructive hover:bg-destructive/10 hover:text-destructive"
                  onClick={handleInterrupt}
                  title="中断当前回合"
                >
                  <StopCircle size={18} />
                </Button>
              ) : null}
              <Button
                size="icon"
                disabled={!composerText.trim() || isSubmitting || !canInteract}
                onClick={handleSend}
                className="size-10 rounded-xl"
              >
                <Send size={18} />
              </Button>
            </div>
          </div>
          <div className="mx-auto mt-3 flex max-w-4xl items-center justify-between px-2 text-xs text-muted-foreground">
            <div className="flex items-center gap-1.5">
              <Zap size={14} className="text-warning" />
              <span>Shift + 回车换行 · 回车发送</span>
            </div>
            {!canInteract ? (
              <span className="text-destructive">连接未就绪,请前往设备页检查</span>
            ) : selectedThread ? (
              <span className="mono">thread {selectedThread.thread_id.slice(0, 8)}</span>
            ) : null}
          </div>
        </div>
      </main>
    </div>
  )
}
