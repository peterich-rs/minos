import { useEffect, useState } from 'react'
import {
  ArrowRight,
  Bot,
  Cpu,
  Shield,
  Sparkles,
  TriangleAlert,
} from 'lucide-react'

import { cn } from '@/shared/lib/utils'
import { useAppStore } from '@/lib/store'
import { exchangeSupabaseSession } from '@/lib/minos'
import {
  getSupabaseAccessToken,
  isSupabaseConfigured,
  signInWithGoogle,
  signInWithSupabasePassword,
  signUpWithSupabasePassword,
} from '@/lib/supabase'

const HIGHLIGHTS = [
  {
    icon: Cpu,
    title: '多主机统一控制面',
    copy: '远程查看会话与审批，与 Desktop 同一账号体系。',
  },
  {
    icon: Bot,
    title: 'Buzz 级壳层体验',
    copy: '渐变 chrome、悬浮 content 面、mauve 导航高亮。',
  },
  {
    icon: Shield,
    title: 'Supabase → Minos',
    copy: 'IdP 只负责身份；业务 JWT 仍由 Minos 签发。',
  },
]

export function AuthScreen() {
  const { deviceId, authMode, setAuthMode, setSession } = useAppStore()
  const [email, setEmail] = useState('')
  const [password, setPassword] = useState('')
  const [confirmPassword, setConfirmPassword] = useState('')
  const [authBusy, setAuthBusy] = useState(false)
  const [authError, setAuthError] = useState<string | null>(null)
  const supabaseReady = isSupabaseConfigured()

  const passwordReady = password.length >= 8
  const confirmReady = password === confirmPassword && passwordReady
  const disabled =
    authBusy ||
    !supabaseReady ||
    !email.includes('@') ||
    !passwordReady ||
    (authMode === 'register' && !confirmReady)

  useEffect(() => {
    if (!supabaseReady) return
    let cancelled = false
    ;(async () => {
      try {
        const token = await getSupabaseAccessToken()
        if (!token || cancelled) return
        setAuthBusy(true)
        setAuthError(null)
        const response = await exchangeSupabaseSession(deviceId, token, 'Minos Web')
        if (cancelled) return
        setSession({
          accountId: response.account.account_id,
          email: response.account.email,
          accessToken: response.access_token,
          refreshToken: response.refresh_token,
        })
      } catch (error) {
        if (!cancelled) {
          setAuthError(error instanceof Error ? error.message : String(error))
        }
      } finally {
        if (!cancelled) setAuthBusy(false)
      }
    })()
    return () => {
      cancelled = true
    }
  }, [deviceId, setSession, supabaseReady])

  async function applyMinosSessionFromSupabaseToken(supabaseAccessToken: string) {
    const response = await exchangeSupabaseSession(
      deviceId,
      supabaseAccessToken,
      'Minos Web',
    )
    setSession({
      accountId: response.account.account_id,
      email: response.account.email,
      accessToken: response.access_token,
      refreshToken: response.refresh_token,
    })
  }

  async function handleSubmit() {
    if (disabled) return
    if (!supabaseReady) {
      setAuthError(
        'Supabase is required (set VITE_SUPABASE_URL and VITE_SUPABASE_ANON_KEY).',
      )
      return
    }
    setAuthBusy(true)
    setAuthError(null)
    try {
      const supabaseToken =
        authMode === 'register'
          ? await signUpWithSupabasePassword(email, password)
          : await signInWithSupabasePassword(email, password)
      await applyMinosSessionFromSupabaseToken(supabaseToken)
    } catch (error) {
      setAuthError(error instanceof Error ? error.message : String(error))
    } finally {
      setAuthBusy(false)
    }
  }

  async function handleGoogle() {
    if (!supabaseReady || authBusy) return
    setAuthBusy(true)
    setAuthError(null)
    try {
      await signInWithGoogle()
    } catch (error) {
      setAuthError(error instanceof Error ? error.message : String(error))
      setAuthBusy(false)
    }
  }

  return (
    <div className="relative grid min-h-full grid-cols-1 overflow-hidden lg:grid-cols-[1fr_440px]">
      <div className="minos-theme-gradient" aria-hidden />
      <div className="minos-theme-grain" aria-hidden />

      <section className="relative z-10 flex flex-col justify-between p-10 lg:p-16">
        <div>
          <div className="inline-flex items-center gap-1.5 rounded-full border border-ink/10 bg-surface/80 px-3 py-1 text-2xs font-semibold text-ink-secondary shadow-sm backdrop-blur-md">
            <Sparkles className="h-3 w-3 text-primary" />
            Minos cloud
          </div>
          <h1 className="mt-6 max-w-xl text-4xl font-semibold leading-[1.08] tracking-tight text-ink lg:text-[2.75rem]">
            在浏览器里查看你的
            <span className="text-primary"> AI 主机与会话。</span>
          </h1>
          <p className="mt-4 max-w-md text-sm leading-relaxed text-ink-secondary">
            壳层对标 Buzz：渐变 chrome、悬浮内容面。功能层逐步接 CloudPort。
          </p>
        </div>
        <div className="mt-12 grid max-w-xl grid-cols-1 gap-3 sm:grid-cols-3">
          {HIGHLIGHTS.map((item) => {
            const Icon = item.icon
            return (
              <div
                key={item.title}
                className="rounded-2xl border border-ink/8 bg-surface/75 p-4 shadow-panel backdrop-blur-md"
              >
                <Icon className="mb-2 h-4 w-4 text-primary" />
                <h3 className="text-sm font-semibold text-ink">{item.title}</h3>
                <p className="mt-1 text-2xs leading-relaxed text-ink-muted">
                  {item.copy}
                </p>
              </div>
            )
          })}
        </div>
      </section>

      <section className="relative z-10 flex items-center justify-center p-6 lg:p-10">
        <div className="w-full max-w-sm rounded-2xl border border-ink/8 bg-surface p-8 shadow-shell">
          <div className="mb-6">
            <div className="mb-3 flex h-10 w-10 items-center justify-center rounded-2xl bg-ink text-surface shadow-sm">
              <Sparkles className="h-5 w-5" />
            </div>
            <h2 className="text-xl font-semibold tracking-tight text-ink">
              {authMode === 'login' ? '登录到 Minos' : '创建账户'}
            </h2>
            <p className="mt-1 text-sm text-ink-muted">
              {supabaseReady
                ? 'Supabase 身份 → Minos 业务会话'
                : '需要配置 Supabase（VITE_SUPABASE_URL / ANON_KEY）'}
            </p>
          </div>

          {supabaseReady ? (
            <div className="mb-5 space-y-3">
              <button
                type="button"
                disabled={authBusy}
                onClick={() => void handleGoogle()}
                className="flex h-10 w-full items-center justify-center rounded-lg border border-ink/10 bg-surface text-sm font-medium text-ink shadow-sm transition-colors hover:bg-surface-hover disabled:opacity-50"
              >
                使用 Google 登录
              </button>
              <div className="flex items-center gap-3 text-2xs text-ink-muted">
                <div className="h-px flex-1 bg-ink/8" />
                或使用邮箱
                <div className="h-px flex-1 bg-ink/8" />
              </div>
            </div>
          ) : null}

          <div className="mb-4 grid grid-cols-2 gap-1 rounded-lg bg-surface-muted p-1">
            {(['login', 'register'] as const).map((mode) => (
              <button
                key={mode}
                type="button"
                onClick={() => {
                  setAuthMode(mode)
                  setAuthError(null)
                }}
                className={cn(
                  'rounded-md py-2 text-sm font-medium transition-colors',
                  authMode === mode
                    ? 'bg-surface text-ink shadow-sm'
                    : 'text-ink-muted hover:text-ink',
                )}
              >
                {mode === 'login' ? '登录' : '注册'}
              </button>
            ))}
          </div>

          <form
            className="space-y-3"
            onSubmit={(e) => {
              e.preventDefault()
              void handleSubmit()
            }}
          >
            <label className="block space-y-1.5">
              <span className="text-2xs font-medium text-ink-secondary">邮箱</span>
              <input
                value={email}
                onChange={(e) => setEmail(e.target.value)}
                type="email"
                autoComplete="email"
                required
                className="h-10 w-full rounded-lg border border-ink/10 bg-surface px-3 text-sm text-ink outline-none ring-primary/30 placeholder:text-ink-muted focus:ring-2"
                placeholder="you@example.com"
              />
            </label>
            <label className="block space-y-1.5">
              <span className="text-2xs font-medium text-ink-secondary">密码</span>
              <input
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                type="password"
                autoComplete={
                  authMode === 'login' ? 'current-password' : 'new-password'
                }
                required
                className="h-10 w-full rounded-lg border border-ink/10 bg-surface px-3 text-sm text-ink outline-none ring-primary/30 placeholder:text-ink-muted focus:ring-2"
                placeholder="至少 8 个字符"
              />
            </label>
            {authMode === 'register' ? (
              <label className="block space-y-1.5">
                <span className="text-2xs font-medium text-ink-secondary">
                  确认密码
                </span>
                <input
                  value={confirmPassword}
                  onChange={(e) => setConfirmPassword(e.target.value)}
                  type="password"
                  required
                  className="h-10 w-full rounded-lg border border-ink/10 bg-surface px-3 text-sm text-ink outline-none ring-primary/30 focus:ring-2"
                  placeholder="再次输入"
                />
              </label>
            ) : null}

            {authError ? (
              <div className="flex items-start gap-2 rounded-lg border border-status-failed/25 bg-status-failed/5 px-3 py-2.5 text-sm text-status-failed">
                <TriangleAlert className="mt-0.5 h-4 w-4 shrink-0" />
                <span>{authError}</span>
              </div>
            ) : null}

            <button
              type="submit"
              disabled={disabled}
              className="flex h-10 w-full items-center justify-center gap-1.5 rounded-lg bg-primary text-sm font-semibold text-white shadow-sm transition-opacity hover:opacity-90 disabled:opacity-40"
            >
              {authBusy
                ? '处理中…'
                : authMode === 'login'
                  ? '登录'
                  : '创建账户'}
              <ArrowRight className="h-4 w-4" />
            </button>
          </form>

          <p className="mt-5 text-center text-2xs text-ink-muted">
            device {deviceId.slice(0, 8)} · browser-admin
          </p>
        </div>
      </section>
    </div>
  )
}
