import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: './tests',
  workers: 1,
  timeout: 45_000,
  expect: { timeout: 10_000 },
  use: {
    channel: 'chromium',
    baseURL: 'http://127.0.0.1:8087/site/',
    viewport: { width: 1440, height: 1000 },
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
    launchOptions: { args: ['--enable-unsafe-swiftshader', '--use-angle=swiftshader'] },
  },
  webServer: {
    command: 'python3 -m http.server 8087 --bind 127.0.0.1 --directory ../../target',
    url: 'http://127.0.0.1:8087/site/',
    reuseExistingServer: false,
  },
});
