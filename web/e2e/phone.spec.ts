// On a phone (390 x 844): nothing wider than the screen, one pane at a
// time. The sections are a row over the list; a chosen file's card is the
// whole screen with a way back; the viewer, the share dialog and the
// public link page fit.
import { expect, test, type Locator, type Page } from "@playwright/test";
import en from "../src/locales/en.js";

test.use({ viewport: { width: 390, height: 844 } });
test.describe.configure({ mode: "serial" });

const home = (page: Page) => page.locator("ortus-drive-home");
const file = (page: Page) => page.locator("ortus-drive-file");

async function fits(page: Page, app?: Locator): Promise<void> {
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(390);
  if (app) {
    const [scroll, client] = await app.evaluate((el) => [el.scrollWidth, el.clientWidth]);
    expect(scroll).toBeLessThanOrEqual(client);
  }
}

/** The element takes (nearly) the app's whole width. */
async function fullWidth(app: Locator, part: Locator): Promise<void> {
  const outer = await app.boundingBox();
  const inner = await part.boundingBox();
  expect(inner && outer && inner.width >= outer.width - 32).toBe(true);
}

test("the drive on a phone: the list, a file's card with a way back, the viewer and sharing", async ({ page, browser }) => {
  await page.goto("/drive");
  const app = home(page);
  await app.getByRole("button", { name: en.newFolder }).click();
  const dialog = app.getByRole("dialog", { name: en.newFolder });
  await dialog.getByLabel(en.folderName).fill("Road repairs");
  expect((await dialog.boundingBox())!.width).toBeLessThanOrEqual(390);
  await dialog.getByRole("button", { name: en.create }).click();
  await app.getByRole("button", { name: "Road repairs" }).click();
  await expect(app.getByRole("heading", { level: 1 })).toContainText("Road repairs");
  await app.locator("input.files").setInputFiles([
    { name: "bridge-inspection-report-for-the-northern-district-2026.txt", mimeType: "text/plain", buffer: Buffer.from("Bridge inspection\n\nThe north span needs new bearings before winter.") },
    { name: "detour-map.png", mimeType: "image/png", buffer: Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==", "base64") },
  ]);
  const table = app.getByRole("table");
  await expect(table.getByRole("row")).toHaveCount(3);
  await fits(page, app);
  await page.screenshot({ path: "test-results/phone-drive-list.png" });

  // A file's card: the whole screen, then back to the list.
  const name = "bridge-inspection-report-for-the-northern-district-2026.txt";
  await table.getByRole("button", { name }).click();
  const detail = app.getByRole("region", { name });
  await expect(detail).toContainText("north span");
  await expect(table).toBeHidden();
  await fullWidth(app, detail);
  await fits(page, app);
  await page.screenshot({ path: "test-results/phone-drive-detail.png" });
  await detail.getByRole("button", { name: en.back }).click();
  await expect(table).toBeVisible();
  await expect(detail).toBeHidden();

  // The viewer and the share dialog.
  await table.getByRole("button", { name }).click();
  await detail.getByRole("button", { name: en.open }).click();
  await expect(file(page).getByRole("article")).toContainText("north span");
  await page.screenshot({ path: "test-results/phone-drive-viewer.png" });
  await fits(page, file(page));
  await file(page).getByRole("button", { name: en.share, exact: true }).click();
  const share = file(page).getByRole("dialog", { name: `Share “${name}”` });
  await share.getByLabel(en.label).selectOption("public");
  await share.getByRole("button", { name: en.makeLink }).click();
  await expect(share.getByText(en.anyoneWithLink)).toBeVisible();
  expect((await share.boundingBox())!.width).toBeLessThanOrEqual(390);
  await fits(page);
  await page.screenshot({ path: "test-results/phone-drive-share.png" });
  const url = await share.locator(".url span").textContent();
  await share.getByRole("button", { name: en.done }).click();
  await file(page).getByRole("button", { name: en.back }).click();
  await expect(app.getByRole("table")).toBeVisible();

  // The link, opened on a phone without an account.
  const visitor = await browser.newContext({ storageState: { cookies: [], origins: [] }, viewport: { width: 390, height: 844 } });
  const outside = await visitor.newPage();
  await outside.goto(`http://${url}`);
  const pub = outside.locator("ortus-drive-public");
  await expect(pub.getByRole("heading", { level: 1 })).toHaveText(name);
  await expect(pub.getByRole("article")).toContainText("north span");
  await fits(outside);
  await outside.screenshot({ path: "test-results/phone-drive-public.png" });
  await visitor.close();
});
