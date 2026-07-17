import { motion, AnimatePresence } from 'framer-motion'
import { Brain, Bot, User, ShieldAlert } from 'lucide-react'
import type { TranscriptItem } from '@/lib/chat-utils'
import { ToolCallBadge } from './tool-call-badge'
import { Avatar, AvatarFallback } from '@/components/ui/avatar'
import { formatClock } from '@/lib/chat-utils'

export function ChatTranscript({ items }: { items: TranscriptItem[] }) {
  return (
    <div className="flex flex-col gap-4 w-full max-w-4xl mx-auto py-8 px-4">
      <AnimatePresence initial={false}>
        {items.map((item) => {
          if (item.kind === 'system') {
            return (
              <motion.div
                key={item.id}
                initial={{ opacity: 0, y: 10 }}
                animate={{ opacity: 1, y: 0 }}
                className="flex flex-col items-center justify-center my-4"
              >
                <div className="flex items-center gap-2 px-4 py-2 rounded-full bg-muted/50 border border-border text-xs text-muted-foreground font-mono">
                  <ShieldAlert size={14} />
                  <span>{item.text}</span>
                </div>
              </motion.div>
            )
          }

          if (item.kind === 'user') {
            return (
              <motion.div
                key={item.id}
                layout
                initial={{ opacity: 0, y: 20 }}
                animate={{ opacity: 1, y: 0 }}
                className="flex gap-4 w-full flex-row-reverse"
              >
                <Avatar className="w-8 h-8 border border-primary/20 bg-primary/10">
                  <AvatarFallback className="bg-transparent text-foreground">
                    <User size={16} />
                  </AvatarFallback>
                </Avatar>
                <div className="flex flex-col gap-2 max-w-[85%] items-end">
                  <div className="flex items-center gap-2 text-xs text-muted-foreground">
                    <span className="font-medium text-foreground">You</span>
                    <span>{formatClock(item.startedAtMs)}</span>
                  </div>
                  <div className="px-5 py-3.5 rounded-2xl whitespace-pre-wrap leading-relaxed text-[15px] shadow-sm bg-primary text-primary-foreground rounded-tr-sm">
                    {item.text}
                  </div>
                </div>
              </motion.div>
            )
          }

          if (item.kind === 'reasoning') {
            return (
              <motion.div
                key={item.id}
                layout
                initial={{ opacity: 0, y: 12 }}
                animate={{ opacity: 1, y: 0 }}
                className="flex gap-4 w-full flex-row"
              >
                <Avatar className="w-8 h-8 border border-border bg-card shadow-sm">
                  <AvatarFallback className="bg-transparent text-foreground">
                    <Bot size={16} />
                  </AvatarFallback>
                </Avatar>
                <div className="flex flex-col gap-2 max-w-[85%] items-start w-full">
                  <div className="w-full bg-muted/30 border border-border rounded-2xl rounded-tl-sm p-4 text-sm text-muted-foreground">
                    <div className="flex items-center gap-2 mb-2 text-xs font-mono uppercase tracking-widest text-primary/70 font-semibold">
                      <Brain size={14} className="animate-pulse" />
                      Thinking
                    </div>
                    <div className="whitespace-pre-wrap font-mono text-[13px] leading-relaxed">
                      {item.text}
                    </div>
                  </div>
                </div>
              </motion.div>
            )
          }

          if (item.kind === 'tool') {
            return (
              <motion.div
                key={item.id}
                layout
                initial={{ opacity: 0, y: 12 }}
                animate={{ opacity: 1, y: 0 }}
                className="flex gap-4 w-full flex-row"
              >
                <div className="w-8" />
                <div className="flex flex-wrap gap-2 mt-1 w-full justify-start max-w-[85%]">
                  <ToolCallBadge toolCall={item.toolCall} />
                </div>
              </motion.div>
            )
          }

          if (item.kind === 'assistant_text') {
            return (
              <motion.div
                key={item.id}
                layout
                initial={{ opacity: 0, y: 20 }}
                animate={{ opacity: 1, y: 0 }}
                className="flex gap-4 w-full flex-row"
              >
                <Avatar className="w-8 h-8 border border-border bg-card shadow-sm">
                  <AvatarFallback className="bg-transparent text-foreground">
                    <Bot size={16} />
                  </AvatarFallback>
                </Avatar>
                <div className="flex flex-col gap-2 max-w-[85%] items-start">
                  <div className="flex items-center gap-2 text-xs text-muted-foreground">
                    <span className="font-medium text-foreground">Agent</span>
                  </div>
                  <div className="px-5 py-3.5 rounded-2xl whitespace-pre-wrap leading-relaxed text-[15px] shadow-sm bg-card border border-border text-foreground rounded-tl-sm">
                    {item.text}
                    {item.showCursor ? (
                      <span className="inline-block w-1.5 h-4 ml-1 align-middle bg-primary/70 animate-pulse" />
                    ) : null}
                  </div>
                </div>
              </motion.div>
            )
          }

          // placeholder
          return (
            <motion.div
              key={item.id}
              layout
              initial={{ opacity: 0, y: 12 }}
              animate={{ opacity: 1, y: 0 }}
              className="flex gap-4 w-full flex-row"
            >
              <Avatar className="w-8 h-8 border border-border bg-card shadow-sm">
                <AvatarFallback className="bg-transparent text-foreground">
                  <Bot size={16} />
                </AvatarFallback>
              </Avatar>
              <div className="flex gap-1 items-center px-4 py-3 rounded-2xl rounded-tl-sm bg-card border border-border">
                <span className="w-1.5 h-1.5 rounded-full bg-primary/60 animate-bounce [animation-delay:-0.3s]" />
                <span className="w-1.5 h-1.5 rounded-full bg-primary/60 animate-bounce [animation-delay:-0.15s]" />
                <span className="w-1.5 h-1.5 rounded-full bg-primary/60 animate-bounce" />
              </div>
            </motion.div>
          )
        })}
      </AnimatePresence>
    </div>
  )
}
