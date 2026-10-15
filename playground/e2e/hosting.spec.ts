import { test, expect } from "@playwright/test";
import lzstring from "lz-string";

test("the static build loads beneath a directory, including worker and WASM assets", async ({
  page,
  context,
}) => {
  await context.route("**/nested/**", async (route) => {
    const response = await route.fetch({ url: route.request().url().replace("/nested/", "/") });
    await route.fulfill({ response });
  });
  await page.goto(
    "/nested/#code/" + lzstring.compressToEncodedURIComponent("export const value = 1;"),
  );
  await expect(page.locator("#compiler-status")).toHaveText("0 diagnostics");
  await expect(page.locator(".playground-plugin-container")).toContainText(
    "export const value = 1",
  );
  await context.unrouteAll({ behavior: "wait" });
});

test("dragging resizes the sidebar and the editor remains usable", async ({ page }) => {
  await page.goto("/#code/" + lzstring.compressToEncodedURIComponent("const value = 1;"));
  await expect(page.locator("#compiler-status")).toHaveText("0 diagnostics");
  const sidebar = page.locator(".playground-sidebar");
  const before = (await sidebar.boundingBox())!;
  const dragbar = (await page.locator(".playground-dragbar:not(.left)").boundingBox())!;
  await page.mouse.move(dragbar.x + dragbar.width / 2, dragbar.y + 100);
  await page.mouse.down();
  await page.mouse.move(dragbar.x + 70, dragbar.y + 100);
  await page.mouse.up();
  expect((await sidebar.boundingBox())!.width).not.toBe(before.width);
  await page.locator("#sidebar-toggle").click();
  await expect(sidebar).not.toBeVisible();
  await page.locator("#sidebar-toggle").click();
  await expect(sidebar).toBeVisible();
});

test("reselecting a tab does not leave an inactive pane subscribed during edits", async ({
  page,
}) => {
  await page.addInitScript(() => localStorage.setItem("enable-sidebar-ast", "true"));
  await page.goto("/#code/" + lzstring.compressToEncodedURIComponent("const value = 1;"));
  await expect(page.locator("#compiler-status")).toHaveText("0 diagnostics");
  await page.getByRole("tab", { name: ".JS", exact: true }).click();
  await page.getByRole("tab", { name: ".JS", exact: true }).click();
  await page.getByRole("tab", { name: "AST" }).click();
  await page.locator(".monaco-editor").click({ position: { x: 120, y: 30 } });
  await page.keyboard.press("ControlOrMeta+A");
  await page.keyboard.insertText("const changed = 2;");
  await expect(page.locator("#compiler-status")).toHaveText("Checking…");
  await expect(page.locator("#compiler-status")).toHaveText("0 diagnostics");
  await expect(page.locator(".playground-plugin-container")).toContainText("AST/type inspection");
});
