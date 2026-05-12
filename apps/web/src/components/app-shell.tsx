import { AnimatePresence, motion } from 'framer-motion'

import { useAppStore, type RouteKey } from '@/lib/store'

import { AppSidebar } from './app-sidebar'
import { ChatWorkspace } from './chat-workspace'
import { DevicesWorkspace } from './devices-workspace'
import { FriendsWorkspace } from './friends-workspace'
import { ProfileWorkspace } from './profile-workspace'
import { SettingsWorkspace } from './settings-workspace'
import { TasksWorkspace } from './tasks-workspace'

function renderRoute(route: RouteKey) {
  switch (route) {
    case 'chat':
      return <ChatWorkspace />
    case 'tasks':
      return <TasksWorkspace />
    case 'friends':
      return <FriendsWorkspace />
    case 'devices':
      return <DevicesWorkspace />
    case 'profile':
      return <ProfileWorkspace />
    case 'settings':
      return <SettingsWorkspace />
    default:
      return null
  }
}

export function AppShell() {
  const { route } = useAppStore()

  return (
    <div className="flex h-screen w-full bg-background">
      <AppSidebar />
      <div className="relative flex flex-1 flex-col overflow-hidden">
        <AnimatePresence mode="wait">
          <motion.main
            key={route}
            initial={{ opacity: 0, y: 8 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: -8 }}
            transition={{ duration: 0.22, ease: [0.2, 0.8, 0.2, 1] }}
            className="flex h-full flex-1 flex-col overflow-hidden"
          >
            {renderRoute(route)}
          </motion.main>
        </AnimatePresence>
      </div>
    </div>
  )
}
