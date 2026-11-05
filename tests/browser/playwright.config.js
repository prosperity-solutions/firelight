// Browser e2e (§13.3). `firelight preview` serves the acme fixtures; the
// binary comes from FIRELIGHT_BIN (CI builds it first) or target/debug.
const { defineConfig, devices } = require("@playwright/test");
const path = require("node:path");

const root = path.resolve(__dirname, "../..");
const bin = process.env.FIRELIGHT_BIN || path.join(root, "target/debug/firelight");
const port = Number(process.env.FIRELIGHT_TEST_PORT || 18399);

module.exports = defineConfig({
  testDir: "./specs",
  timeout: 30_000,
  retries: 0,
  workers: 1,
  reporter: [["list"]],
  use: { baseURL: `http://127.0.0.1:${port}`, trace: "retain-on-failure" },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  webServer: {
    command: `"${bin}" preview fixtures/acme-infra fixtures/acme-app --embedder off --listen 127.0.0.1:${port}`,
    cwd: root,
    url: `http://127.0.0.1:${port}/readyz`,
    timeout: 60_000,
    reuseExistingServer: false,
  },
});
