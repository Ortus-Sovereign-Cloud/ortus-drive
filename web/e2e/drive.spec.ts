// The drive (boards WebDrive and WebFilePreview): folders, uploads with
// their progress, a document's preview, versions, sharing with a
// colleague, a label that allows a link, and the link opened without an
// account, with its password.
import { expect, test, type Page } from "@playwright/test";
import en from "../src/locales/en.js";

test.describe.configure({ mode: "serial" });

const home = (page: Page) => page.locator("ortus-drive-home");
const file = (page: Page) => page.locator("ortus-drive-file");
const headers = { "X-Requested-With": "ortus" };

test.beforeAll(async ({ request }) => {
  await request.post("/api/v1/admin/users", { headers, data: { email: "marek@ministry.example", name: "Marek Nowak", role: "member", groups: ["flood-team"], language: "pl" } });
});

test("folders and uploads", async ({ page }) => {
  await page.goto("/drive");
  await home(page).getByRole("button", { name: en.newFolder }).click();
  const dialog = home(page).getByRole("dialog", { name: en.newFolder });
  await dialog.getByLabel(en.folderName).fill("Shelters");
  await dialog.getByRole("button", { name: en.create }).click();
  await home(page).getByRole("button", { name: "Shelters" }).click();
  await expect(home(page).getByRole("heading", { level: 1 })).toContainText("Shelters");
  await expect(home(page).getByRole("status")).toHaveText(en.emptyFolder);
  await home(page).locator("input.files").setInputFiles([
    { name: "shelter-agreement.txt", mimeType: "text/plain", buffer: Buffer.from("Shelter agreement\n\nClause 4: the school on Park Street is the second site.") },
    { name: "site-photo.png", mimeType: "image/png", buffer: Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==", "base64") },
  ]);
  await expect(home(page).getByRole("status").filter({ hasText: /uploaded/ })).toBeVisible();
  const table = home(page).getByRole("table");
  await expect(table.getByRole("row")).toHaveCount(3);
  await expect(table).toContainText("shelter-agreement.txt");
  await expect(table).toContainText(en.onlyYou);
  await table.getByRole("button", { name: "shelter-agreement.txt" }).click();
  const detail = home(page).getByRole("region", { name: "shelter-agreement.txt" });
  await expect(detail).toContainText("Clause 4");
  await page.screenshot({ path: "test-results/drive.png" });
});

test("a document opens with its preview and versions", async ({ page }) => {
  await page.goto("/drive");
  await home(page).getByRole("button", { name: "Shelters" }).click();
  await home(page).getByRole("table").getByRole("button", { name: "shelter-agreement.txt" }).dblclick();
  await expect(file(page).getByRole("heading", { level: 1 })).toHaveText("shelter-agreement.txt");
  await expect(file(page).getByRole("article")).toContainText("the school on Park Street");
  await file(page).getByRole("button", { name: en.versions }).click();
  await expect(file(page).getByRole("dialog", { name: en.versions })).toContainText("Version 1");
});

test("sharing follows the label; the link opens without an account", async ({ page, browser }) => {
  await page.goto("/drive");
  await home(page).getByRole("button", { name: "Shelters" }).click();
  await home(page).getByRole("table").getByRole("button", { name: "shelter-agreement.txt" }).dblclick();
  await file(page).getByRole("button", { name: en.share, exact: true }).click();
  const share = file(page).getByRole("dialog", { name: "Share “shelter-agreement.txt”" });
  await share.getByLabel(en.addPeople).fill("marek@ministry.example");
  await share.getByRole("button", { name: en.add, exact: true }).click();
  await expect(share.getByRole("list", { name: en.whoHasAccess })).toContainText("Marek Nowak");
  // INTERNAL (the default) allows no links.
  await share.getByRole("button", { name: en.makeLink }).click();
  await expect(share.getByRole("alert")).toContainText("cannot be shared this way");
  // PUBLIC does.
  await share.getByLabel(en.label).selectOption("public");
  await share.getByRole("button", { name: en.makeLink }).click();
  await expect(share.getByText(en.anyoneWithLink)).toBeVisible();
  await share.getByLabel(en.password).fill("river-gauge-42");
  await share.getByLabel(en.password).press("Tab");
  await expect(share.getByLabel(en.password)).toHaveAttribute("placeholder", en.on);
  await page.screenshot({ path: "test-results/share.png" });
  const url = await share.locator(".url span").textContent();
  await share.getByRole("button", { name: en.done }).click();

  const visitor = await browser.newContext({ storageState: { cookies: [], origins: [] } });
  const outside = await visitor.newPage();
  await outside.goto(`http://${url}`);
  const pub = outside.locator("ortus-drive-public");
  await pub.getByLabel(en.linkPassword).fill("wrong-password");
  await pub.getByRole("button", { name: en.openLink }).click();
  await expect(pub.getByRole("alert")).toHaveText(en.wrongPassword);
  await pub.getByLabel(en.linkPassword).fill("river-gauge-42");
  await pub.getByRole("button", { name: en.openLink }).click();
  await expect(pub.getByRole("heading", { level: 1 })).toHaveText("shelter-agreement.txt");
  await expect(pub.getByRole("article")).toContainText("Clause 4");
  await expect(pub.getByRole("button", { name: en.download })).toBeVisible();
  await visitor.close();
});

test("trash and back", async ({ page }) => {
  await page.goto("/drive");
  await home(page).getByRole("button", { name: "Shelters" }).click();
  const table = home(page).getByRole("table");
  await table.getByRole("button", { name: "site-photo.png" }).click();
  await home(page).getByRole("region", { name: "site-photo.png" }).getByRole("button", { name: en.moveToTrash }).click();
  await expect(table.getByRole("row")).toHaveCount(2);
  await home(page).getByRole("navigation").getByRole("button", { name: en.trash }).click();
  await home(page).getByRole("table").getByRole("button", { name: en.restore }).click();
  await expect(home(page).getByRole("status")).toHaveText(en.trashEmpty);
});
