import { defineConfig } from 'vitest/config'

// The editor needs a DOM: jsdom stands in for WebKit in the tests. What
// only a real WebKit shows (spelling, Writing Tools, keys typed by the
// system) is checked by the app's UI scenarios (`make ui-test`).
export default defineConfig({
  test: {
    environment: 'jsdom',
    include: ['test/**/*.test.ts'],
    setupFiles: ['test/setup.ts'],
  },
})
