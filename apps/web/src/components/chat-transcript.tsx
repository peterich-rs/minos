import { motion, AnimatePresence } from 'framer-motion'
import { Brain, Bot, User, ShieldAlert } from 'lucide-react'
import type { TranscriptMessage } from '@/lib/chat-utils'
import { ToolCallBadge } from './tool-call-badge'
import { Avatar, AvatarFallback } from '@/components/ui/avatar'
import { formatClock } from '@/lib/chat-utils'

export function ChatTranscript({ messages }: { messages: TranscriptMessage[] }) {
  return (
    <div className="flex flex-col gap-6 w-full max-w-4xl mx-auto py-8 px-4">
      <AnimatePresence initial={false}>
        {messages.map((msg) => {
          const isUser = msg.role === 'user'
          const isSystem = msg.role === 'system'
          
          if (isSystem) {
            return (
              <motion.div
                key={msg.messageId}
                initial={{ opacity: 0, y: 10 }}
                animate={{ opacity: 1, y: 0 }}
                className="flex flex-col items-center justify-center my-4"
              >
                <div className="flex items-center gap-2 px-4 py-2 rounded-full bg-muted/50 border border-border text-xs text-muted-foreground font-mono">
                  <ShieldAlert size={14} />
                  <span>{msg.text}</span>
                </div>
              </motion.div>
            )
          }

          return (
            <motion.div
              key={msg.messageId}
              layout
              initial={{ opacity: 0, y: 20 }}
              animate={{ opacity: 1, y: 0 }}
              className={`flex gap-4 w-full ${isUser ? 'flex-row-reverse' : 'flex-row'}`}
            >
              <Avatar className={`w-8 h-8 border ${isUser ? 'border-primary/20 bg-primary/10' : 'border-border bg-card shadow-sm'}`}>
                <AvatarFallback className="bg-transparent text-foreground">
                  {isUser ? <User size={16} /> : <Bot size={16} />}
                </AvatarFallback>
              </Avatar>
              
              <div className={`flex flex-col gap-2 max-w-[85%] ${isUser ? 'items-end' : 'items-start'}`}>
                <div className="flex items-center gap-2 text-xs text-muted-foreground">
                  <span className="font-medium text-foreground">{isUser ? 'You' : 'Agent'}</span>
                  <span>{formatClock(msg.startedAtMs)}</span>
                </div>

                {/* Reasoning Box (if available) */}
                {msg.reasoning && (
                  <motion.div 
                    initial={{ opacity: 0, height: 0 }}
                    animate={{ opacity: 1, height: 'auto' }}
                    className="w-full bg-muted/30 border border-border rounded-2xl rounded-tl-sm p-4 text-sm text-muted-foreground"
                  >
                    <div className="flex items-center gap-2 mb-2 text-xs font-mono uppercase tracking-widest text-primary/70 font-semibold">
                      <Brain size={14} className="animate-pulse" />
                      Thinking
                    </div>
                    <div className="whitespace-pre-wrap font-mono text-[13px] leading-relaxed">
                      {msg.reasoning}
                    </div>
                  </motion.div>
                )}

                {/* Tool Calls */}
                {msg.toolCalls.length > 0 && (
                  <div className="flex flex-wrap gap-2 mt-1 w-full justify-start">
                    {msg.toolCalls.map((tc) => (
                      <ToolCallBadge key={tc.toolCallId} toolCall={tc} />
                    ))}
                  </div>
                )}

                {/* Main Text Content */}
                {msg.text && (
                  <div 
                    className={`px-5 py-3.5 rounded-2xl whitespace-pre-wrap leading-relaxed text-[15px] shadow-sm
                      ${isUser 
                        ? 'bg-primary text-primary-foreground rounded-tr-sm' 
                        : 'bg-card border border-border text-foreground rounded-tl-sm'
                      }`}
                  >
                    {msg.text}
                  </div>
                )}

                {/* Loading indicator if message is not complete yet but has no content */}
                {!msg.finishedAtMs && !msg.text && msg.toolCalls.length === 0 && !msg.reasoning && (
                  <div className="flex gap-1 items-center px-4 py-3 rounded-2xl rounded-tl-sm bg-card border border-border">
                    <span className="w-1.5 h-1.5 rounded-full bg-primary/60 animate-bounce [animation-delay:-0.3s]" />
                    <span className="w-1.5 h-1.5 rounded-full bg-primary/60 animate-bounce [animation-delay:-0.15s]" />
                    <span className="w-1.5 h-1.5 rounded-full bg-primary/60 animate-bounce" />
                  </div>
                )}
              </div>
            </motion.div>
          )
        })}
      </AnimatePresence>
    </div>
  )
}
