import { createRequire } from 'node:module'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

const __dirname = path.dirname(fileURLToPath(import.meta.url))
const require = createRequire(import.meta.url)

/** Package root under web node_modules (pnpm-safe). */
function webPkgDir(name: string): string {
  let dir = path.dirname(require.resolve(name))
  while (dir !== path.dirname(dir)) {
    const pkgPath = path.join(dir, 'package.json')
    if (fs.existsSync(pkgPath)) {
      try {
        const pkg = JSON.parse(fs.readFileSync(pkgPath, 'utf8')) as {
          name?: string
        }
        if (pkg.name === name) return dir
      } catch {
        // keep walking
      }
    }
    dir = path.dirname(dir)
  }
  throw new Error(`web package root not found: ${name}`)
}

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
    // Shared chrome is source-aliased from apps/desktop; pin React so we do not
    // load a second copy if desktop node_modules is also present locally.
    dedupe: ['react', 'react-dom'],
    // Array + exact regex: object aliases can prefix-match and break subpaths
    // (e.g. react → index.js then react/jsx-runtime → index.js/jsx-runtime).
    alias: [
      // More specific first: desktop shared must not resolve via web `@`.
      {
        find: '@/shared',
        replacement: path.resolve(__dirname, '../desktop/src/shared'),
      },
      {
        find: '@shared',
        replacement: path.resolve(__dirname, '../desktop/src/shared'),
      },
      { find: '@', replacement: path.resolve(__dirname, './src') },
      // Bare imports in desktop shared walk node_modules from apps/desktop.
      // CI installs only web deps — pin shared peers to this package.
      { find: /^react$/, replacement: webPkgDir('react') },
      { find: /^react-dom$/, replacement: webPkgDir('react-dom') },
      { find: /^clsx$/, replacement: webPkgDir('clsx') },
      { find: /^tailwind-merge$/, replacement: webPkgDir('tailwind-merge') },
      { find: /^lucide-react$/, replacement: webPkgDir('lucide-react') },
      {
        find: 'react/jsx-runtime',
        replacement: require.resolve('react/jsx-runtime'),
      },
      {
        find: 'react/jsx-dev-runtime',
        replacement: require.resolve('react/jsx-dev-runtime'),
      },
    ],
  },
  server: {
    allowedHosts,
  },
  preview: {
    allowedHosts,
  },
})
