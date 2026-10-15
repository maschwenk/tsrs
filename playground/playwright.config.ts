import { defineConfig } from "@playwright/test";
import process from "node:process";

const hostedURL = process.env.PLAYWRIGHT_BASE_URL;

export default defineConfig({
  testDir: "./e2e",
  workers: 1,
  timeout: hostedURL ? 60_000 : 30_000,
  expect: { timeout: hostedURL ? 15_000 : 5_000 },
  use: {
    baseURL: hostedURL || "http://127.0.0.1:4173",
    headless: true,
    launchOptions: { executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE },
  },
  webServer: hostedURL
    ? undefined
    : {
        command: "vp preview --host 127.0.0.1 --port 4173",
        url: "http://127.0.0.1:4173",
        reuseExistingServer: !process.env.CI,
      },
});
