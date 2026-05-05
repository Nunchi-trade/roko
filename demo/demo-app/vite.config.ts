import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

export default defineConfig({
  plugins: [react()],
  base: '/',
  build: {
    outDir: 'dist',
    sourcemap: false,
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (id.includes('/node_modules/three/')) return 'vendor-three';
          if (id.includes('/node_modules/@xterm/')) return 'vendor-xterm';
        },
      },
    },
  },
  server: {
    proxy: {
      '/api': 'http://localhost:6677',
      '/ws': { target: 'http://localhost:6677', ws: true },
      '/health': 'http://localhost:6677',
    },
    // Prevent Vite from watching directories that roko-serve or CLI commands
    // write to during demo execution, which triggers unwanted full page reloads.
    watch: {
      ignored: [
        '**/node_modules/**',
        '**/.roko/**',
        '**/roko.toml',
        '**/target/**',
        '/tmp/**',
      ],
    },
    hmr: {
      // Overlay compile errors only — don't let transient HMR disconnects
      // (e.g. roko-serve restart) trigger a full browser reload.
      overlay: true,
    },
  },
});
