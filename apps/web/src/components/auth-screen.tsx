import { useState } from 'react'
import { motion, AnimatePresence } from 'framer-motion'
import {
  ArrowRight,
  Bot,
  CheckCircle2,
  Cpu,
  Shield,
  Sparkles,
  TriangleAlert,
} from 'lucide-react'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Tabs, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { Badge } from '@/components/ui/badge'
import { useAppStore } from '@/lib/store'
import { loginBrowserAccount, registerBrowserAccount } from '@/lib/minos'

const HIGHLIGHTS = [
  {
    icon: Cpu,
    title: '多主机统一控制面',
    copy: '把所有配对的 Mac 集中管理,切换 Runtime 只需一次点击。',
  },
  {
    icon: Bot,
    title: '一次发起,全程可观测',
    copy: '实时查看工具调用、推理过程与输出,随时中断重来。',
  },
  {
    icon: Shield,
    title: '端到端账号体系',
    copy: '刷新令牌自动续签,修改密码会撤销其他设备的会话。',
  },
]

export function AuthScreen() {
  const { deviceId, authMode, setAuthMode, setSession } = useAppStore()
  const [email, setEmail] = useState('')
  const [password, setPassword] = useState('')
  const [confirmPassword, setConfirmPassword] = useState('')
  const [authBusy, setAuthBusy] = useState(false)
  const [authError, setAuthError] = useState<string | null>(null)

  const passwordReady = password.length >= 8
  const confirmReady = password === confirmPassword && passwordReady
  const disabled =
    authBusy ||
    !email.includes('@') ||
    !passwordReady ||
    (authMode === 'register' && !confirmReady)

  async function handleSubmit() {
    if (disabled) return
    setAuthBusy(true)
    setAuthError(null)
    try {
      const response =
        authMode === 'register'
          ? await registerBrowserAccount(deviceId, email, password)
          : await loginBrowserAccount(deviceId, email, password)
      setSession({
        accountId: response.account.account_id,
        email: response.account.email,
        accessToken: response.access_token,
        refreshToken: response.refresh_token,
      })
    } catch (error) {
      setAuthError(error instanceof Error ? error.message : String(error))
    } finally {
      setAuthBusy(false)
    }
  }

  return (
    <div className="relative grid min-h-screen grid-cols-1 bg-background lg:grid-cols-[1fr_440px]">
      {/* Left — hero */}
      <motion.section
        initial={{ opacity: 0, y: 20 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.45, ease: [0.2, 0.8, 0.2, 1] }}
        className="relative flex flex-col justify-between overflow-hidden gradient-surface p-10 lg:p-16"
      >
        <div className="pointer-events-none absolute -right-[20%] top-[5%] size-[60%] rounded-full bg-primary/20 blur-[140px]" />
        <div className="pointer-events-none absolute -left-[10%] bottom-[5%] size-[40%] rounded-full bg-primary/10 blur-[140px]" />

        <div className="relative z-10">
          <Badge variant="outline" className="mono">
            <Sparkles size={12} />
            Minos web console
          </Badge>
          <h1 className="mt-6 max-w-2xl text-4xl font-bold leading-[1.05] tracking-tight lg:text-6xl">
            在浏览器里驾驭你的每一台
            <span className="bg-gradient-to-r from-primary to-primary/60 bg-clip-text text-transparent">
              {' '}
              AI 开发主机。
            </span>
          </h1>
          <p className="mt-5 max-w-md text-base text-muted-foreground">
            配对 Mac、发起 Agent 回合、跟进工具调用。不把工作流压成仪表盘表格,保留编码的直觉。
          </p>
        </div>

        <div className="relative z-10 mt-12 grid max-w-xl grid-cols-1 gap-4 sm:grid-cols-3">
          {HIGHLIGHTS.map((item) => {
            const Icon = item.icon
            return (
              <motion.div
                key={item.title}
                initial={{ opacity: 0, y: 12 }}
                animate={{ opacity: 1, y: 0 }}
                transition={{ delay: 0.15, duration: 0.3 }}
                className="rounded-xl border border-border/60 bg-card/60 p-4 backdrop-blur-md"
              >
                <Icon size={18} className="mb-3 text-primary" />
                <h3 className="text-sm font-semibold">{item.title}</h3>
                <p className="mt-1 text-[12px] leading-relaxed text-muted-foreground">
                  {item.copy}
                </p>
              </motion.div>
            )
          })}
        </div>
      </motion.section>

      {/* Right — form */}
      <motion.section
        initial={{ opacity: 0, y: 20 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ delay: 0.08, duration: 0.45, ease: [0.2, 0.8, 0.2, 1] }}
        className="flex items-center justify-center border-l border-border/60 bg-card p-8 lg:p-12"
      >
        <div className="w-full max-w-sm space-y-8">
          <div>
            <div className="mb-3 flex size-11 items-center justify-center rounded-2xl bg-primary text-primary-foreground shadow-lg">
              <Sparkles size={20} />
            </div>
            <h2 className="text-2xl font-bold tracking-tight">
              {authMode === 'login' ? '登录到 Minos' : '创建 Minos 账户'}
            </h2>
            <p className="mt-1 text-sm text-muted-foreground">
              {authMode === 'login'
                ? '使用你的邮箱和密码继续操作。'
                : '邮箱会成为账户标识,密码至少 8 位。'}
            </p>
          </div>

          <Tabs
            value={authMode}
            onValueChange={(v) => {
              setAuthMode(v as 'login' | 'register')
              setAuthError(null)
            }}
          >
            <TabsList className="grid w-full grid-cols-2 rounded-full bg-muted p-1">
              <TabsTrigger
                value="login"
                className="rounded-full data-[state=active]:bg-background data-[state=active]:shadow-sm"
              >
                登录
              </TabsTrigger>
              <TabsTrigger
                value="register"
                className="rounded-full data-[state=active]:bg-background data-[state=active]:shadow-sm"
              >
                注册
              </TabsTrigger>
            </TabsList>
          </Tabs>

          <form
            className="space-y-4"
            onSubmit={(e) => {
              e.preventDefault()
              void handleSubmit()
            }}
          >
            <div className="space-y-1.5">
              <Label>邮箱</Label>
              <Input
                value={email}
                onChange={(e) => setEmail(e.target.value)}
                placeholder="you@example.com"
                type="email"
                autoComplete="email"
                required
                className="h-11"
              />
            </div>
            <div className="space-y-1.5">
              <Label>密码</Label>
              <Input
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                placeholder="至少 8 个字符"
                type="password"
                autoComplete={authMode === 'login' ? 'current-password' : 'new-password'}
                required
                className="h-11"
              />
            </div>
            <AnimatePresence initial={false}>
              {authMode === 'register' ? (
                <motion.div
                  initial={{ opacity: 0, height: 0 }}
                  animate={{ opacity: 1, height: 'auto' }}
                  exit={{ opacity: 0, height: 0 }}
                  className="overflow-hidden space-y-1.5"
                >
                  <Label>确认密码</Label>
                  <Input
                    value={confirmPassword}
                    onChange={(e) => setConfirmPassword(e.target.value)}
                    placeholder="再次输入"
                    type="password"
                    required
                    className="h-11"
                  />
                </motion.div>
              ) : null}
            </AnimatePresence>

            {authMode === 'register' ? (
              <div className="flex flex-wrap gap-2 pt-1">
                <Badge variant={passwordReady ? 'success' : 'secondary'}>
                  {passwordReady ? <CheckCircle2 size={12} /> : null}8+ 字符
                </Badge>
                <Badge variant={confirmReady ? 'success' : 'secondary'}>
                  {confirmReady ? <CheckCircle2 size={12} /> : null}两次一致
                </Badge>
              </div>
            ) : null}

            <AnimatePresence>
              {authError ? (
                <motion.div
                  initial={{ opacity: 0, y: -4 }}
                  animate={{ opacity: 1, y: 0 }}
                  exit={{ opacity: 0 }}
                  className="flex items-center gap-2 rounded-lg border border-destructive/30 bg-destructive/10 px-3 py-2.5 text-sm text-destructive"
                >
                  <TriangleAlert size={16} />
                  <span>{authError}</span>
                </motion.div>
              ) : null}
            </AnimatePresence>

            <Button type="submit" disabled={disabled} className="h-11 w-full text-sm font-semibold">
              {authBusy ? '处理中…' : authMode === 'login' ? '登录' : '创建账户'}
              <ArrowRight size={16} className="ml-1" />
            </Button>
          </form>

          <p className="text-center text-xs text-muted-foreground">
            device {deviceId.slice(0, 8)} · role browser-admin
          </p>
        </div>
      </motion.section>
    </div>
  )
}
