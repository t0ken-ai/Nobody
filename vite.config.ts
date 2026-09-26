import { defineConfig } from 'vite';

// A fixed loopback origin lets the desktop CSP stay narrow in development.
export default defineConfig({
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  build: { target: 'es2022' },
});
