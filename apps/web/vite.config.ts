import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import path from 'path'

const allowedHosts = ['vibe.fan-nn.top']

if (process.env.VITE_ALLOWED_HOSTS) {
  allowedHosts.push(
    ...process.env.VITE_ALLOWED_HOSTS.split(',')
      .map((host) => host.trim())
      .filter(Boolean),
  )
}

// https://vite.dev/config/
export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src'),
    },
  },
  server: {
    allowedHosts,
  },
  preview: {
    allowedHosts,
  },
})
