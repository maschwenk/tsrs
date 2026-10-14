import { test, expect } from "@playwright/test";
import lzstring from "lz-string";

const share = (source: string, options = "") =>
  "/" + options + "#code/" + lzstring.compressToEncodedURIComponent(source);

test("tsrs checks, emits, shares, and runs code without a JS TypeScript compiler", async ({
  page,
}) => {
  const errors: string[] = [];
  const requests: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("request", (request) => requests.push(request.url()));
  await page.goto(share('const greeting: string = "Hello from tsrs";\nconsole.log(greeting);'));
  await expect(page.locator("#compiler-status")).toHaveText("0 diagnostics");
  await expect(page.locator(".playground-plugin-container")).toContainText(
    'const greeting = "Hello from tsrs"',
  );
  await page.getByRole("tab", { name: ".D.TS" }).click();
  await expect(page.locator(".playground-plugin-container")).toContainText("greeting: string");
  await page.getByText("Run", { exact: true }).click();
  await expect(page.locator("#log")).toContainText("Hello from tsrs");
  await page.locator('select[name="colours"]').selectOption("force-dark");
  await expect(page.locator("html")).toHaveClass(/dark-theme/);
  await expect(page.locator("#compiler-status")).toHaveText("0 diagnostics");
  await page.getByText("Share", { exact: true }).click();
  await expect(page).toHaveURL(/#code\//);
  await page.reload();
  await expect(page.locator("#compiler-status")).toHaveText("0 diagnostics");
  expect(
    requests.some(
      (url) =>
        url.includes("/vs/language/typescript/") || url.includes("/typescript/lib/typescript"),
    ),
  ).toBe(false);
  expect(errors).toEqual([]);
});

test("multi-file errors and emitted files are selectable; examples and config work", async ({
  page,
}) => {
  await page.goto(
    share(
      '// @filename: a.ts\nexport const value = "😀";\n// @filename: b.ts\nimport { value } from "./a";\nexport const wrong: number = value;\n',
    ),
  );
  await expect(page.locator("#compiler-status")).toHaveText("1 diagnostic");
  const outputs = page.getByLabel("JavaScript output file");
  await expect(outputs.locator("option")).toHaveCount(2);
  await page.getByRole("tab", { name: /Errors/ }).click();
  await expect(page.locator(".compiler-diagnostics")).toContainText("not assignable");
  await page.getByText("TS Config", { exact: true }).first().click();
  await page.locator("#option-strict").uncheck();
  await page.getByRole("button", { name: "Close", exact: true }).first().click();
  await page.getByText("Examples", { exact: true }).first().click();
  await expect(
    page.locator("#examples .section-content.button-typescript .example-link").first(),
  ).toBeVisible();
  await page.getByRole("tab", { name: "JavaScript", exact: true }).click();
  await page.locator("#examples .example-link").filter({ hasText: "Hello World" }).first().click();
  await expect(page.locator("#compiler-status")).not.toHaveText("Checking…");
  await page.getByText("Settings", { exact: true }).first().click();
  await expect(page.locator(".playground-settings-container")).toBeVisible();
});

test("a failed WASM load clears output and the same worker can retry", async ({ page }) => {
  await page.route("**/*.wasm", (route) =>
    route.fulfill({ status: 503, body: "temporarily unavailable" }),
  );
  await page.goto(share("export const value = 1;"));
  await expect(page.locator("#compiler-status")).toHaveText("Compiler failed — Retry");
  await expect(page.locator(".playground-plugin-container")).toContainText("503");
  await expect(page.getByLabel("JavaScript output file")).toHaveCount(0);
  await page.unroute("**/*.wasm");
  await page.locator("#compiler-retry").click();
  await expect(page.locator("#compiler-status")).toHaveText("0 diagnostics");
  await expect(page.locator(".playground-plugin-container")).toContainText(
    "export const value = 1",
  );
});

test("plugin lifecycle failures are visible without breaking checking or emit", async ({
  page,
}) => {
  await page.route("https://unpkg.com/tsrs-smoke-plugin@latest/dist/index.js", (route) =>
    route.fulfill({
      contentType: "text/javascript",
      body: 'define([], function () { return { id: "smoke", displayName: "Smoke plugin", didMount: async function (sandbox) { await sandbox.getAST(); } }; });',
    }),
  );
  page.on("dialog", (dialog) => dialog.accept());
  await page.goto(share("export const value = 1;", "?install-plugin=tsrs-smoke-plugin"));
  await expect(page.locator("#compiler-status")).toHaveText("0 diagnostics");
  await page.getByRole("tab", { name: "Smoke plugin" }).click();
  await expect(page.locator(".playground-plugin-container")).toContainText(
    "AST inspection is unavailable",
  );
  await page.getByRole("tab", { name: ".JS", exact: true }).click();
  await expect(page.locator(".playground-plugin-container")).toContainText(
    "export const value = 1",
  );
  await page.reload();
  await expect(page.getByRole("tab", { name: "Smoke plugin" })).toBeVisible();
});

test("exports use current compiler settings and emit-disabled state", async ({ page }) => {
  await page.goto(share("const value = 1;"));
  await expect(page.locator("#compiler-status")).toHaveText("0 diagnostics");
  await page.getByText("TS Config", { exact: true }).first().click();
  await page.locator("#option-strict").uncheck();
  await page.getByRole("button", { name: "Close", exact: true }).first().click();
  await page.evaluate(() => (window as any).sandbox.updateCompilerSetting("noEmit", true));
  await expect(page.locator(".playground-plugin-container")).toContainText("No files emitted");
  await page.evaluate(() => (window as any).sandbox.updateCompilerSetting("noEmit", false));
  await page.getByText("Export", { exact: true }).click();
  await page.getByText("Copy as Markdown Issue", { exact: true }).click();
  await expect(page.locator("#popover-modal textarea")).toContainText('"strict": false');
  await expect(page.locator("#popover-modal textarea")).toContainText(
    '"moduleResolution": "bundler"',
  );
});
