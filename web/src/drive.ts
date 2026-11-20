// The drive (board WebDrive): one's own drive and its folders, what others
// share, recent, starred, files saved from mail and the trash; uploads in
// parts with their progress; the selected file's card; the quota.
import { ApiError, LitElement, LocalizeController, call, controls, css, customElement, html, nothing, repeat, state } from "ortus-kit";
import { t } from "./i18n.js";
import type { DriveKey } from "./locales/en.js";

export interface Item {
  id: string;
  parent: string;
  kind: "folder" | "file";
  name: string;
  size: number;
  mime: string;
  version: number;
  label: string | null;
  source: string | null;
  note?: string | null;
  owner: string | null;
  mine: boolean;
  modifiedAt: number;
  modifiedBy: string | null;
  trashedAt: number | null;
  role: "view" | "comment" | "edit" | "owner" | null;
  access?: { link: boolean; people: number; groups: string[] };
}

type Section = "mine" | "shared" | "recent" | "starred" | "email" | "trash";

const SECTIONS: [Section, DriveKey][] = [
  ["mine", "myDrive"],
  ["shared", "sharedWithMe"],
  ["recent", "recent"],
  ["starred", "starred"],
  ["email", "fromEmail"],
  ["trash", "trash"],
];

interface Upload {
  name: string;
  size: number;
  sent: number;
  failed: boolean;
  done: boolean;
}

/** The short kind of a file, as its badge shows it. */
export function badge(item: { kind: string; name: string }): string {
  if (item.kind === "folder") return "DIR";
  const ext = (item.name.split(".").pop() ?? "").toLowerCase();
  const map: Record<string, string> = { docx: "DOC", doc: "DOC", odt: "DOC", pdf: "PDF", xlsx: "XLS", xls: "XLS", ods: "XLS", csv: "XLS", pptx: "PPT", odp: "PPT", zip: "ZIP", png: "IMG", jpg: "IMG", jpeg: "IMG", gif: "IMG", webp: "IMG", txt: "TXT", md: "TXT" };
  return map[ext] ?? (ext.slice(0, 3).toUpperCase() || "FILE");
}

export function sizeText(bytes: number): string {
  const lang = document.documentElement.lang || "en";
  const [value, unit] = bytes >= 1e9 ? [bytes / 1e9, "gigabyte"] : bytes >= 1e6 ? [bytes / 1e6, "megabyte"] : [Math.max(1, Math.round(bytes / 1000)), "kilobyte"];
  return new Intl.NumberFormat(lang, { style: "unit", unit, unitDisplay: "short", maximumFractionDigits: 1 }).format(value);
}

export function whenText(ms: number): string {
  const lang = document.documentElement.lang || "en";
  const d = new Date(ms);
  const today = new Date();
  today.setHours(0, 0, 0, 0);
  if (ms >= today.getTime()) return new Intl.DateTimeFormat(lang, { hour: "2-digit", minute: "2-digit", hourCycle: "h23" }).format(d);
  if (ms >= today.getTime() - 6 * 86_400_000) return new Intl.DateTimeFormat(lang, { weekday: "short" }).format(d);
  return new Intl.DateTimeFormat(lang, { day: "numeric", month: "short" }).format(d);
}

export function go(path: string): void {
  history.pushState(null, "", path);
  window.dispatchEvent(new PopStateEvent("popstate"));
}

@customElement("ortus-drive-home")
export class OrtusDriveHome extends LitElement {
  static override styles = [
    controls,
    css`
      :host {
        display: grid;
        grid-template-rows: auto 1fr;
        min-block-size: 100vh;
        color: var(--ortus-text);
      }
      header {
        display: flex;
        align-items: center;
        gap: 16px;
        padding-block: 14px;
        padding-inline: 24px;
        border-block-end: 1px solid var(--ortus-line);
      }
      .search {
        inline-size: min(586px, 50vw);
      }
      .body {
        display: grid;
        grid-template-columns: 196px minmax(0, 1fr) 256px;
        gap: 28px;
        padding-block: 20px;
        padding-inline: 12px 24px;
        align-items: start;
      }
      aside.side {
        display: flex;
        flex-direction: column;
        gap: 4px;
        min-block-size: calc(100vh - 110px);
      }
      .make {
        display: flex;
        gap: 8px;
        margin-block-end: 20px;
      }
      .make .primary {
        flex: 1;
        block-size: 44px;
      }
      .make .plus {
        inline-size: 44px;
        block-size: 44px;
        padding: 0;
        font-size: 20px;
      }
      nav button {
        display: flex;
        align-items: center;
        justify-content: space-between;
        inline-size: 100%;
        block-size: 40px;
        padding-inline: 12px;
        border-radius: 10px;
        border: 1px solid transparent;
        background: transparent;
        color: var(--ortus-text);
        font: inherit;
        font-size: 15px;
        cursor: pointer;
        text-align: start;
      }
      nav button[aria-current="page"] {
        background: var(--ortus-hover);
        border-color: var(--ortus-line-strong);
        font-weight: 600;
      }
      nav small {
        color: var(--ortus-text-3);
        font-size: 13px;
      }
      .quota {
        margin-block-start: auto;
        display: grid;
        gap: 8px;
        padding-inline: 12px;
        font-size: 13px;
        color: var(--ortus-text-3);
      }
      .bar {
        block-size: 4px;
        border-radius: 2px;
        background: var(--ortus-line-strong);
        overflow: hidden;
      }
      .bar span {
        display: block;
        block-size: 100%;
        background: var(--ortus-accent);
      }
      .top {
        display: flex;
        align-items: center;
        gap: 12px;
        margin-block-end: 20px;
      }
      .crumbs {
        display: flex;
        align-items: center;
        gap: 8px;
        margin: 0;
        font-size: 20px;
        font-weight: 600;
        flex: 1;
        min-inline-size: 0;
      }
      .crumbs button {
        border: 0;
        background: none;
        color: var(--ortus-text-2);
        font: inherit;
        cursor: pointer;
        padding: 0;
      }
      .crumbs span[aria-current] {
        color: var(--ortus-text);
      }
      .views {
        display: inline-flex;
        gap: 4px;
        padding: 4px;
        border-radius: 12px;
        border: 1px solid var(--ortus-line-strong);
      }
      .views button {
        inline-size: 36px;
        block-size: 32px;
        border: 0;
        border-radius: 8px;
        background: transparent;
        color: var(--ortus-text-3);
        cursor: pointer;
        font: inherit;
      }
      .views button[aria-pressed="true"] {
        background: var(--ortus-hover);
        color: var(--ortus-text);
      }
      h2 {
        margin-block: 0 12px;
        margin-inline: 0;
        font-size: 14px;
        font-weight: 500;
        color: var(--ortus-text-2);
      }
      .suggested {
        display: grid;
        grid-template-columns: repeat(3, minmax(0, 1fr));
        gap: 16px;
        margin-block-end: 24px;
      }
      .card {
        display: flex;
        align-items: center;
        gap: 14px;
        padding: 16px;
        border-radius: 16px;
        border: 1px solid var(--ortus-line);
        background: var(--ortus-surface);
        color: inherit;
        font: inherit;
        text-align: start;
        cursor: pointer;
        min-inline-size: 0;
      }
      .card strong,
      .name {
        display: block;
        font-weight: 600;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      .card small {
        color: var(--ortus-text-3);
        font-size: 13px;
      }
      .badge {
        flex: none;
        display: grid;
        place-items: center;
        inline-size: 40px;
        block-size: 40px;
        border-radius: 10px;
        background: var(--ortus-surface-2);
        color: var(--ortus-accent);
        font-size: 11px;
        font-weight: 700;
      }
      .badge.PDF {
        color: var(--ortus-red);
      }
      .badge.XLS {
        color: var(--ortus-teal);
      }
      .badge.ZIP,
      .badge.IMG {
        color: var(--ortus-orange);
      }
      table {
        inline-size: 100%;
        border-collapse: separate;
        border-spacing: 0;
        border-radius: 18px;
        border: 1px solid var(--ortus-line);
        background: var(--ortus-surface);
        overflow: hidden;
      }
      th {
        text-align: start;
        font-size: 13px;
        font-weight: 500;
        color: var(--ortus-text-2);
        padding-block: 14px;
        padding-inline: 20px;
        border-block-end: 1px solid var(--ortus-line);
      }
      td {
        padding-block: 10px;
        padding-inline: 20px;
        border-block-end: 1px solid var(--ortus-line);
        font-size: 14px;
        color: var(--ortus-text-2);
        white-space: nowrap;
      }
      tr:last-child td {
        border-block-end: 0;
      }
      tr.selected td {
        background: var(--ortus-hover);
      }
      td.first {
        color: var(--ortus-text);
      }
      td.first button {
        display: flex;
        align-items: center;
        gap: 14px;
        border: 0;
        background: none;
        color: inherit;
        font: inherit;
        font-size: 15px;
        font-weight: 600;
        cursor: pointer;
        padding: 0;
        max-inline-size: 360px;
        text-align: start;
      }
      .shared {
        color: var(--ortus-orange);
      }
      .grid {
        display: grid;
        grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
        gap: 16px;
      }
      .tile {
        display: grid;
        gap: 10px;
        padding: 12px;
        border-radius: 16px;
        border: 1px solid var(--ortus-line);
        background: var(--ortus-surface);
        color: inherit;
        font: inherit;
        text-align: start;
        cursor: pointer;
      }
      .tile img,
      .tile .ph {
        inline-size: 100%;
        aspect-ratio: 4 / 3;
        object-fit: cover;
        border-radius: 10px;
        background: var(--ortus-surface-2);
        display: grid;
        place-items: center;
        color: var(--ortus-text-3);
        font-weight: 700;
      }
      .empty {
        padding: 48px 24px;
        border-radius: 18px;
        border: 1px dashed var(--ortus-line-strong);
        text-align: center;
        color: var(--ortus-text-2);
      }
      .detail {
        display: grid;
        gap: 12px;
        padding: 20px;
        border-radius: 18px;
        border: 1px solid var(--ortus-line);
        background: var(--ortus-surface);
      }
      .detail .thumb {
        inline-size: 100%;
        aspect-ratio: 16 / 10;
        border-radius: 10px;
        object-fit: cover;
        background: #eceef3;
      }
      .detail .page {
        aspect-ratio: 16 / 10;
        border-radius: 10px;
        background: #eceef3;
        color: #1b1d24;
        padding: 14px;
        font-size: 8px;
        line-height: 1.4;
        overflow: hidden;
      }
      .detail p {
        margin: 0;
        font-size: 13px;
        color: var(--ortus-text-3);
      }
      .detail h3 {
        margin: 0;
        font-size: 15px;
        overflow-wrap: anywhere;
      }
      .who {
        font-size: 14px;
        color: var(--ortus-text-2);
      }
      .detail .link {
        color: var(--ortus-orange);
      }
      .detail .actions {
        display: flex;
        gap: 8px;
        flex-wrap: wrap;
      }
      .uploads {
        position: fixed;
        inset-block-end: 24px;
        inset-inline-end: 24px;
        inline-size: 360px;
        display: grid;
        gap: 10px;
        padding: 20px;
        border-radius: 18px;
        border: 1px solid var(--ortus-line-strong);
        background: var(--ortus-surface);
        z-index: 5;
      }
      .uploads p {
        margin: 0;
        display: flex;
        justify-content: space-between;
        font-size: 14px;
      }
      .uploads small {
        color: var(--ortus-text-3);
      }
      .drop {
        outline: 2px dashed var(--ortus-accent);
        outline-offset: -8px;
      }
      dialog {
        inline-size: min(440px, 92vw);
        padding: 0;
        border-radius: 18px;
        border: 1px solid var(--ortus-line-strong);
        background: var(--ortus-surface);
        color: var(--ortus-text);
      }
      dialog::backdrop {
        background: rgb(0 0 0 / 0.55);
      }
      form {
        display: grid;
        gap: 14px;
        padding: 24px;
      }
      form h2 {
        margin: 0;
        font-size: 18px;
        color: var(--ortus-text);
      }
      .row {
        display: flex;
        gap: 10px;
        justify-content: flex-end;
      }
      .error {
        color: var(--ortus-red);
        margin: 0;
        font-size: 14px;
      }
      .sr {
        position: absolute;
        inline-size: 1px;
        block-size: 1px;
        overflow: hidden;
        clip-path: inset(50%);
      }
    `,
  ];

  private readonly localize = new LocalizeController(this);
  @state() private section: Section = "mine";
  @state() private folder = "";
  @state() private items: Item[] = [];
  @state() private path: { id: string; name: string }[] = [];
  @state() private myRole = "owner";
  @state() private selected: Item | null = null;
  @state() private detail: { text: string | null; access: { people: { name: string; email: string; role: string; you?: boolean }[]; groups: { name: string }[]; link: { url: string; expiresAt: number | null } | null } | null } | null = null;
  @state() private suggested: Item[] = [];
  @state() private uploads: Upload[] = [];
  @state() private quota: { used: number; quota: number | null } | null = null;
  @state() private sharedCount = 0;
  @state() private grid = false;
  @state() private error = "";
  @state() private dropping = false;
  @state() private query = "";
  @state() private found: Item[] | null = null;

  override connectedCallback(): void {
    super.connectedCallback();
    const p = new URLSearchParams(location.search);
    const s = p.get("section");
    if (SECTIONS.some(([id]) => id === s)) this.section = s as Section;
    this.folder = p.get("folder") ?? "";
    try {
      this.grid = localStorage.getItem("ortus.drive.grid") === "1";
    } catch {
      /* no storage */
    }
    void this.load();
  }

  private place(): void {
    const url = new URL(location.href);
    url.search = "";
    if (this.section !== "mine") url.searchParams.set("section", this.section);
    if (this.folder) url.searchParams.set("folder", this.folder);
    history.replaceState(history.state, "", url);
  }

  private async load(): Promise<void> {
    this.error = "";
    try {
      if (this.section === "mine") {
        const r = await call<{ items: Item[]; path: { id: string; name: string }[]; role: string }>("GET", `/drive/items${this.folder ? `?parent=${encodeURIComponent(this.folder)}` : ""}`);
        this.items = r.items;
        this.path = r.path;
        this.myRole = r.role;
      } else if (this.section === "shared") {
        this.items = (await call<{ items: Item[] }>("GET", "/drive/shared")).items;
        this.path = [];
      } else {
        const which = this.section === "email" ? "email" : this.section;
        this.items = (await call<{ items: Item[] }>("GET", `/drive/special?which=${which}`)).items;
        this.path = [];
      }
      const [recent, quota, shared] = await Promise.all([
        call<{ items: Item[] }>("GET", "/drive/special?which=recent"),
        call<{ used: number; quota: number | null }>("GET", "/drive/quota"),
        call<{ items: Item[] }>("GET", "/drive/shared"),
      ]);
      this.suggested = recent.items.slice(0, 3);
      this.quota = quota;
      this.sharedCount = shared.items.length;
      if (this.selected && !this.items.some((i) => i.id === this.selected?.id)) this.select(null);
    } catch (e) {
      this.error = e instanceof ApiError ? e.message : String(e);
    }
  }

  private show(section: Section): void {
    this.section = section;
    this.folder = "";
    this.found = null;
    this.select(null);
    this.place();
    void this.load();
  }

  private openFolder(id: string): void {
    this.section = "mine";
    this.folder = id;
    this.found = null;
    this.select(null);
    this.place();
    void this.load();
  }

  private open(item: Item): void {
    if (item.kind === "folder") this.openFolder(item.id);
    else go(`/drive?file=${encodeURIComponent(item.id)}`);
  }

  private select(item: Item | null): void {
    this.selected = item;
    this.detail = null;
    if (!item) return;
    void Promise.all([
      item.kind === "file" && !item.mime.startsWith("image/") ? call<{ text: string | null }>("GET", `/drive/items/${item.id}/text`).catch(() => ({ text: null })) : Promise.resolve({ text: null }),
      call<{ people: { name: string; email: string; role: string }[]; groups: { name: string }[]; link: { url: string; expiresAt: number | null } | null }>("GET", `/drive/items/${item.id}/shares`).catch(() => null),
    ]).then(([text, access]) => {
      if (this.selected?.id === item.id) this.detail = { text: text.text, access };
    });
  }

  private async search(): Promise<void> {
    const q = this.query.trim();
    if (!q) {
      this.found = null;
      return;
    }
    try {
      const r = await call<{ results: { id: string; title: string; meta: { kind: string; size: number; mime: string }; at: number; mine: boolean }[] }>("GET", `/search?app=drive&q=${encodeURIComponent(q)}`);
      this.found = r.results.map((h) => ({ id: h.id, parent: "", kind: h.meta.kind === "folder" ? "folder" : "file", name: h.title, size: h.meta.size ?? 0, mime: h.meta.mime ?? "", version: 0, label: null, source: null, owner: null, mine: h.mine, modifiedAt: h.at, modifiedBy: null, trashedAt: null, role: null }));
    } catch (e) {
      this.error = e instanceof ApiError ? e.message : String(e);
    }
  }

  private async newFolder(e: Event): Promise<void> {
    e.preventDefault();
    const dialog = this.renderRoot.querySelector<HTMLDialogElement>("dialog.folder")!;
    const input = dialog.querySelector<HTMLInputElement>("input")!;
    try {
      await call("POST", "/drive/folders", { parent: this.folder, name: input.value.trim() });
      dialog.close();
      input.value = "";
      await this.load();
    } catch (err) {
      this.error = err instanceof ApiError ? err.message : String(err);
      dialog.close();
    }
  }

  private pick(): void {
    this.renderRoot.querySelector<HTMLInputElement>("input.files")?.click();
  }

  /** Uploads in parts, one file after another. */
  private async upload(files: File[]): Promise<void> {
    const start = this.uploads.length;
    this.uploads = [...this.uploads, ...files.map((f) => ({ name: f.name, size: f.size, sent: 0, failed: false, done: false }))];
    const parent = this.section === "mine" ? this.folder : "";
    for (const [n, file] of files.entries()) {
      const index = start + n;
      const update = (change: Partial<Upload>) => {
        this.uploads = this.uploads.map((u, i) => (i === index ? { ...u, ...change } : u));
      };
      try {
        const begun = await call<{ id: string; partSize: number }>("POST", "/drive/uploads", { parent, name: file.name, size: file.size, mime: file.type });
        const parts = Math.max(1, Math.ceil(file.size / begun.partSize));
        for (let p = 0; p < parts; p++) {
          const chunk = file.slice(p * begun.partSize, (p + 1) * begun.partSize);
          const r = await fetch(`/api/v1/drive/uploads/${begun.id}/parts/${p}`, { method: "PUT", headers: { "X-Requested-With": "ortus", "Content-Type": "application/octet-stream" }, body: chunk });
          if (!r.ok) throw new Error(((await r.json().catch(() => null)) as { error?: { message?: string } } | null)?.error?.message ?? r.statusText);
          update({ sent: Math.min(file.size, (p + 1) * begun.partSize) });
        }
        await call("POST", `/drive/uploads/${begun.id}/finish`);
        update({ done: true, sent: file.size });
      } catch (e) {
        update({ failed: true });
        this.error = e instanceof ApiError ? e.message : e instanceof Error ? e.message : String(e);
      }
    }
    await this.load();
    window.setTimeout(() => {
      if (this.uploads.every((u) => u.done || u.failed)) this.uploads = [];
    }, 4000);
  }

  private async act(item: Item, what: "star" | "unstar" | "trash" | "restore" | "forever"): Promise<void> {
    try {
      if (what === "star" || what === "unstar") await call("PATCH", `/drive/items/${item.id}`, { starred: what === "star" });
      else if (what === "trash") await call("DELETE", `/drive/items/${item.id}`);
      else if (what === "restore") await call("POST", `/drive/items/${item.id}/restore`);
      else await call("DELETE", `/drive/items/${item.id}?forever=1`);
      this.select(null);
      await this.load();
    } catch (e) {
      this.error = e instanceof ApiError ? e.message : String(e);
    }
  }

  private accessText(item: Item): string {
    const a = item.access;
    if (!a) return item.mine ? "" : t(this.localize, "sharedByOwner", { who: item.owner ?? "" });
    if (a.link) return t(this.localize, "linkView");
    if (a.groups.length) return a.groups.join(", ");
    if (a.people) return t(this.localize, "nPeople", { count: a.people + 1 });
    return t(this.localize, "onlyYou");
  }

  override render() {
    const list = this.found ?? this.items;
    const sectionName = t(this.localize, SECTIONS.find(([id]) => id === this.section)?.[1] ?? "myDrive");
    return html`<header>
        <input class="input search" type="search" placeholder=${t(this.localize, "searchFiles")} aria-label=${t(this.localize, "searchFiles")} .value=${this.query} @input=${(e: Event) => (this.query = (e.target as HTMLInputElement).value)} @keydown=${(e: KeyboardEvent) => e.key === "Enter" && this.search()} />
      </header>
      <div class="body ${this.dropping ? "drop" : ""}" @dragover=${(e: DragEvent) => (e.preventDefault(), (this.dropping = true))} @dragleave=${() => (this.dropping = false)} @drop=${(e: DragEvent) => {
        e.preventDefault();
        this.dropping = false;
        const files = [...(e.dataTransfer?.files ?? [])];
        if (files.length) void this.upload(files);
      }}>
        <aside class="side">
          <div class="make">
            <button class="button primary" @click=${this.pick}>${t(this.localize, "upload")}</button>
            <button class="button plus" aria-label=${t(this.localize, "newFolder")} title=${t(this.localize, "newFolder")} @click=${() => this.renderRoot.querySelector<HTMLDialogElement>("dialog.folder")?.showModal()}>+</button>
            <input class="files sr" type="file" multiple aria-hidden="true" tabindex="-1" @change=${(e: Event) => {
              const input = e.target as HTMLInputElement;
              void this.upload([...(input.files ?? [])]);
              input.value = "";
            }} />
          </div>
          <nav aria-label=${t(this.localize, "drive")}>
            ${SECTIONS.map(([id, key]) => html`<button aria-current=${this.section === id && !this.found ? "page" : "false"} @click=${() => this.show(id)}><span>${t(this.localize, key)}</span>${id === "shared" && this.sharedCount ? html`<small>${this.sharedCount}</small>` : nothing}</button>`)}
          </nav>
          ${this.quota
            ? html`<div class="quota">
                ${this.quota.quota
                  ? html`<div class="bar" role="progressbar" aria-valuemin="0" aria-valuemax=${this.quota.quota} aria-valuenow=${this.quota.used}><span .style=${`inline-size: ${Math.min(100, (100 * this.quota.used) / this.quota.quota)}%`}></span></div>
                      <span>${t(this.localize, "usedOf", { used: sizeText(this.quota.used), quota: sizeText(this.quota.quota) })}</span>`
                  : html`<span>${t(this.localize, "used", { used: sizeText(this.quota.used) })}</span>`}
              </div>`
            : nothing}
        </aside>
        <main>
          <div class="top">
            <h1 class="crumbs">
              ${this.found
                ? html`<span aria-current="page">${t(this.localize, "results", { q: this.query })}</span>`
                : this.section === "mine"
                  ? html`${this.folder ? html`<button @click=${() => this.openFolder("")}>${t(this.localize, "myDrive")}</button>` : html`<span aria-current="page">${t(this.localize, "myDrive")}</span>`}
                    ${this.path.map((p, i) => html`<span aria-hidden="true">/</span>${i === this.path.length - 1 ? html`<span aria-current="page">${p.name}</span>` : html`<button @click=${() => this.openFolder(p.id)}>${p.name}</button>`}`)}`
                  : html`<span aria-current="page">${sectionName}</span>`}
            </h1>
            <div class="views" role="group" aria-label=${t(this.localize, "view")}>
              <button aria-pressed=${this.grid ? "false" : "true"} aria-label=${t(this.localize, "listView")} @click=${() => this.setGrid(false)}>☰</button>
              <button aria-pressed=${this.grid ? "true" : "false"} aria-label=${t(this.localize, "gridView")} @click=${() => this.setGrid(true)}>▦</button>
            </div>
            ${this.folder && this.section === "mine" && (this.myRole === "owner" || this.myRole === "edit")
              ? html`<button class="button" @click=${() => go(`/drive?file=${encodeURIComponent(this.folder)}&share=1`)}>${t(this.localize, "shareFolder")}</button>`
              : nothing}
          </div>
          ${this.error ? html`<p class="error" role="alert">${this.error}</p>` : nothing}
          ${this.section === "mine" && !this.folder && !this.found && this.suggested.length
            ? html`<h2>${t(this.localize, "suggested")}</h2>
                <div class="suggested">
                  ${this.suggested.map(
                    (s) => html`<button class="card" @click=${() => this.open(s)}>
                      <span class="badge ${badge(s)}" aria-hidden="true">${badge(s)}</span>
                      <span><strong>${s.name}</strong><small>${s.note ?? (s.modifiedBy ? t(this.localize, "editedBy", { who: s.modifiedBy, when: whenText(s.modifiedAt) }) : whenText(s.modifiedAt))}</small></span>
                    </button>`,
                  )}
                </div>`
            : nothing}
          ${list.length ? (this.grid ? this.renderGrid(list) : this.renderTable(list)) : html`<p class="empty" role="status">${this.found ? t(this.localize, "noResults") : this.section === "trash" ? t(this.localize, "trashEmpty") : t(this.localize, "emptyFolder")}</p>`}
        </main>
        <aside>${this.selected ? this.renderDetail(this.selected) : nothing}</aside>
      </div>
      ${this.uploads.length ? this.renderUploads() : nothing}
      <dialog class="folder" aria-label=${t(this.localize, "newFolder")}>
        <form @submit=${this.newFolder}>
          <h2>${t(this.localize, "newFolder")}</h2>
          <label>${t(this.localize, "folderName")}<input class="input" required maxlength="255" /></label>
          <div class="row">
            <button type="button" class="button" @click=${() => this.renderRoot.querySelector<HTMLDialogElement>("dialog.folder")?.close()}>${t(this.localize, "cancel")}</button>
            <button class="button primary">${t(this.localize, "create")}</button>
          </div>
        </form>
      </dialog>`;
  }

  private setGrid(on: boolean): void {
    this.grid = on;
    try {
      localStorage.setItem("ortus.drive.grid", on ? "1" : "0");
    } catch {
      /* no storage */
    }
  }

  private renderTable(list: Item[]) {
    return html`<table>
      <thead>
        <tr>
          <th scope="col">${t(this.localize, "name")}</th>
          <th scope="col">${t(this.localize, "owner")}</th>
          <th scope="col">${t(this.localize, "modified")}</th>
          <th scope="col">${t(this.localize, "access")}</th>
        </tr>
      </thead>
      <tbody>
        ${repeat(
          list,
          (i) => i.id,
          (i) => html`<tr class=${this.selected?.id === i.id ? "selected" : ""} @click=${() => this.select(i)} @dblclick=${() => this.open(i)}>
            <td class="first">
              <button @click=${(e: Event) => (e.stopPropagation(), i.kind === "folder" ? this.open(i) : this.select(i))} @dblclick=${(e: Event) => (e.stopPropagation(), this.open(i))}>
                <span class="badge ${badge(i)}" aria-hidden="true">${badge(i)}</span><span class="name" dir="auto">${i.name}</span>
              </button>
            </td>
            <td>${i.source === "mail" ? t(this.localize, "fromEmail") : i.mine ? t(this.localize, "you") : (i.owner ?? "")}</td>
            <td>${whenText(i.modifiedAt)}</td>
            <td class=${i.access?.link ? "shared" : ""}>${this.section === "trash" ? html`<button class="button small" @click=${(e: Event) => (e.stopPropagation(), this.act(i, "restore"))}>${t(this.localize, "restore")}</button>` : this.accessText(i)}</td>
          </tr>`,
        )}
      </tbody>
    </table>`;
  }

  private renderGrid(list: Item[]) {
    return html`<div class="grid">
      ${repeat(
        list,
        (i) => i.id,
        (i) => html`<button class="tile" @click=${() => this.select(i)} @dblclick=${() => this.open(i)} aria-label=${i.name}>
          ${i.mime.startsWith("image/") ? html`<img src="/api/v1/drive/items/${i.id}/thumbnail" alt="" loading="lazy" />` : html`<span class="ph" aria-hidden="true">${badge(i)}</span>`}
          <span class="name" dir="auto">${i.name}</span>
        </button>`,
      )}
    </div>`;
  }

  private renderDetail(item: Item) {
    const d = this.detail;
    const people = d?.access?.people ?? [];
    const names = people.map((p) => (p.you ? t(this.localize, "you") : p.name.split(" ")[0])).join(", ");
    return html`<section class="detail" aria-label=${item.name}>
      ${item.mime.startsWith("image/") ? html`<img class="thumb" src="/api/v1/drive/items/${item.id}/thumbnail" alt="" />` : html`<div class="page" aria-hidden="true">${(d?.text ?? "").slice(0, 600)}</div>`}
      <h3 dir="auto">${item.name}</h3>
      <p>${item.kind === "file" ? `${sizeText(item.size)} · ` : ""}${item.modifiedBy ? t(this.localize, "editedBy", { who: item.modifiedBy, when: whenText(item.modifiedAt) }) : whenText(item.modifiedAt)}</p>
      ${people.length ? html`<p>${t(this.localize, "whoHasAccess")}</p><span class="who">${names}${d?.access?.groups.length ? `, ${d.access.groups.map((g) => g.name).join(", ")}` : ""}</span>` : nothing}
      ${d?.access?.link ? html`<span class="link">${t(this.localize, "linkView")}${d.access.link.expiresAt ? ` · ${t(this.localize, "expires", { when: new Intl.DateTimeFormat(document.documentElement.lang || "en", { day: "numeric", month: "short" }).format(new Date(d.access.link.expiresAt)) })}` : ""}</span>` : nothing}
      <div class="actions">
        ${this.section === "trash"
          ? html`<button class="button" @click=${() => this.act(item, "restore")}>${t(this.localize, "restore")}</button>${item.mine ? html`<button class="button" @click=${() => this.act(item, "forever")}>${t(this.localize, "deleteForever")}</button>` : nothing}`
          : html`<button class="button primary" @click=${() => this.open(item)}>${t(this.localize, "open")}</button>
              <button class="button" @click=${() => this.act(item, "star")}>${t(this.localize, "star")}</button>
              ${item.role === "owner" || item.role === "edit" ? html`<button class="button" @click=${() => this.act(item, "trash")}>${t(this.localize, "moveToTrash")}</button>` : nothing}`}
      </div>
    </section>`;
  }

  private renderUploads() {
    const total = this.uploads.reduce((a, u) => a + u.size, 0);
    const sent = this.uploads.reduce((a, u) => a + u.sent, 0);
    const current = this.uploads.find((u) => !u.done && !u.failed) ?? this.uploads[this.uploads.length - 1]!;
    const busy = this.uploads.filter((u) => !u.done && !u.failed).length;
    return html`<section class="uploads" aria-label=${t(this.localize, "uploading", { count: this.uploads.length })} role="status">
      <p><strong>${busy ? t(this.localize, "uploading", { count: this.uploads.length }) : t(this.localize, "uploaded", { count: this.uploads.filter((u) => u.done).length })}</strong></p>
      <p><small>${current.name} · ${sizeText(current.size)} · ${Math.round((100 * current.sent) / Math.max(1, current.size))}%</small></p>
      <div class="bar"><span .style=${`inline-size: ${Math.round((100 * sent) / Math.max(1, total))}%`}></span></div>
    </section>`;
  }
}
