// The drive card of the start page (ortus-drive-today, from today.js):
// the files one worked on last, each a link, and the way into Drive.
import { expect, test } from "@playwright/test";
import en from "../src/locales/en.js";

test("the start page's drive card lists recent files", async ({ page }) => {
  await page.goto("/drive");
  await page.evaluate(async () => {
    const card = "/assets/apps/drive/today.js";
    await import(/* @vite-ignore */ card);
    document.body.append(document.createElement("ortus-drive-today"));
  });
  const card = page.locator("ortus-drive-today").getByRole("region", { name: en.recent });
  await expect(card.getByRole("link", { name: en.todayOpen })).toHaveAttribute("href", "/drive");
  const recent = ((await (await page.request.get("/api/v1/drive/special?which=recent")).json()) as { items: { id: string; kind: string; name: string }[] }).items.filter((i) => i.kind === "file");
  if (recent.length) {
    await expect(card.getByRole("link", { name: new RegExp(recent[0]!.name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")) }).first()).toBeVisible();
  } else {
    await expect(card).toContainText(en.todayNoFiles);
  }
});
