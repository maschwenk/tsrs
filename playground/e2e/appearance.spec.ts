import { test, expect } from "@playwright/test";

test("the official page layout, typography, tabs, and examples are retained", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await expect(page.locator("#compiler-status")).toHaveText("0 diagnostics");
  await expect(page.locator("#top-menu")).toHaveCSS("height", "48px");
  const bounds = (await page.locator("#playground-container").boundingBox())!;
  expect(bounds.y).toBe(96);
  expect(bounds.height).toBe(786);
  expect((await page.locator(".playground-sidebar").boundingBox())!.width).toBe(344);
  const editorFontSize = await page.evaluate(() =>
    navigator.userAgent.includes("Macintosh") ? "12px" : "14px",
  );
  await expect(page.locator(".monaco-editor .view-lines")).toHaveCSS("font-size", editorFontSize);
  await expect(page.locator(".monaco-editor .bracket-highlighting-0").first()).toHaveCSS(
    "color",
    "rgb(4, 49, 250)",
  );
  expect(await page.locator("#playground-plugin-tabbar button").allTextContents()).toEqual([
    ".JS",
    ".D.TS",
    "Errors",
    "Logs",
    "Plugins",
  ]);
  await expect(page.locator(".playground-plugin-container > pre > code")).toBeVisible();
  await expect(page.getByLabel("JavaScript output file")).toHaveCount(0);
  await page.locator("#examples-button").click();
  await expect(page.locator("#examples .section-name.selected")).toHaveText("TypeScript");
  await expect(page.locator("#examples .section-content.button-typescript h4").first()).toHaveText(
    "Primitives",
  );
  await page.getByRole("tab", { name: "JavaScript", exact: true }).click();
  await expect(page.locator("#examples .section-content.selected h4").first()).toHaveText(
    "JavaScript Essentials",
  );
  await page.keyboard.press("Escape");
  await page.locator("#handbook-button").click();
  await expect(page.locator("#navigation-container li")).toHaveCount(12);
  await expect(page.locator("#story-container h2").first()).toHaveText("Compiler Settings");
  await page.getByRole("button", { name: "Close handbook" }).click();
  await page.locator('select[name="colours"]').selectOption("force-dark");
  await expect(page.locator("html")).toHaveClass(/dark-theme/);
  await expect(page.locator("#compiler-status")).toHaveText("0 diagnostics");
  await expect(page.locator("html")).toHaveCSS("--playground-dropdown-bg", "#313131");
});
