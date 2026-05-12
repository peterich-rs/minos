import { motion } from 'framer-motion'
import { Terminal, Code, CheckCircle2, CircleAlert, FileEdit, FileText, Search, Loader2 } from 'lucide-react'
import type { TranscriptToolCall } from '@/lib/chat-utils'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from '@/components/ui/dialog'
import { ScrollArea } from '@/components/ui/scroll-area'

function getToolIcon(name: string) {
  if (name.includes('command') || name.includes('sh')) return <Terminal size={14} />
  if (name.includes('edit') || name.includes('write')) return <FileEdit size={14} />
  if (name.includes('read') || name.includes('view')) return <FileText size={14} />
  if (name.includes('search') || name.includes('grep')) return <Search size={14} />
  return <Code size={14} />
}

export function ToolCallBadge({ toolCall }: { toolCall: TranscriptToolCall }) {
  const isPending = !toolCall.completed
  
  return (
    <Dialog>
      <DialogTrigger asChild>
        <motion.button
          layout
          initial={{ opacity: 0, y: 5, scale: 0.95 }}
          animate={{ opacity: 1, y: 0, scale: 1 }}
          whileHover={{ scale: 1.02 }}
          whileTap={{ scale: 0.98 }}
          className={`flex items-center gap-2 px-3 py-1.5 rounded-full text-xs font-mono border transition-colors max-w-full
            ${toolCall.isError 
              ? 'bg-destructive/10 border-destructive/20 text-destructive hover:bg-destructive/15' 
              : isPending 
                ? 'bg-accent border-accent text-accent-foreground hover:bg-accent/80' 
                : 'bg-background border-border text-muted-foreground hover:bg-muted/50 hover:text-foreground'
            }`}
        >
          {isPending ? (
            <Loader2 size={14} className="animate-spin text-primary" />
          ) : toolCall.isError ? (
            <CircleAlert size={14} />
          ) : (
            getToolIcon(toolCall.name)
          )}
          <span className="truncate max-w-[200px] sm:max-w-[300px]">
            {toolCall.name}
          </span>
          {toolCall.completed && !toolCall.isError && (
            <CheckCircle2 size={14} className="text-emerald-500 ml-1" />
          )}
        </motion.button>
      </DialogTrigger>
      <DialogContent className="max-w-3xl h-[80vh] flex flex-col p-0 gap-0 overflow-hidden bg-background">
        <DialogHeader className="p-4 border-b bg-muted/20">
          <DialogTitle className="flex items-center gap-2 font-mono text-sm">
            {getToolIcon(toolCall.name)}
            {toolCall.name}
          </DialogTitle>
        </DialogHeader>
        <ScrollArea className="flex-1">
          <div className="p-4 space-y-4">
            <div>
              <p className="text-xs font-mono uppercase text-muted-foreground mb-2 tracking-widest">Arguments</p>
              <pre className="p-4 rounded-lg bg-zinc-950 text-zinc-50 font-mono text-sm overflow-x-auto whitespace-pre-wrap">
                {toolCall.argsJson}
              </pre>
            </div>
            {toolCall.completed && (
              <div>
                <p className="text-xs font-mono uppercase text-muted-foreground mb-2 tracking-widest">Output</p>
                <pre className={`p-4 rounded-lg font-mono text-sm overflow-x-auto whitespace-pre-wrap ${
                  toolCall.isError ? 'bg-destructive/10 text-destructive border border-destructive/20' : 'bg-zinc-950 text-zinc-50'
                }`}>
                  {toolCall.output || 'No output.'}
                </pre>
              </div>
            )}
          </div>
        </ScrollArea>
      </DialogContent>
    </Dialog>
  )
}
