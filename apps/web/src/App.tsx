import { AuthScreen } from './components/auth-screen'
import { CloudShell } from './cloud/CloudShell'
import { useAppStore } from './lib/store'

/**
 * Web cloud console — UI chrome SSOT is Desktop (ShellFrame + design-system).
 * Auth: Supabase → Minos exchange; product data mock until CloudPort.
 */
export default function App() {
  const session = useAppStore((s) => s.session)

  return (
    <div className="h-full w-full min-h-0">
      {session ? <CloudShell /> : <AuthScreen />}
    </div>
  )
}
