// A file opened (board WebFilePreview): its preview (images, PDF drawn by
// pdf.js, the text of documents), its versions, sending it by mail, and
// the share dialog with people, groups and the link.
import { ApiError, LitElement, LocalizeController, call, controls, css, customElement, html, nothing, state } from "ortus-kit";
import { badge, go, sizeText, whenText, type Item } from "./drive.js";
import { t } from "./i18n.js";

interface Shares {
  people: { share?: string; name: string; email: string; role: string; you?: boolean }[];
  groups: { share: string; name: string; role: string }[];
  outside: { share: string; email: string; url: string; expiresAt: number | null; opened: number }[];
  link: { share: string; url: string; expiresAt: number | null; password: boolean; noDownload: boolean; opened: number } | null;
  label: string | null;
}

interface Version {
  version: number;
  size: number;
  createdAt: number;
  createdBy: string | null;
}

/** Draws a PDF's pages into canvases (pdf.js, no scripts from the file). */
async function drawPdf(url: string, into: HTMLElement, headers: Record<string, string> = {}): Promise<void> {
  const pdfjs = await import("pdfjs-dist");
  // Beside app.js (see vite.config.ts); a plain string so the bundler leaves it.
  const worker = "pdf.worker.min.js";
  pdfjs.GlobalWorkerOptions.workerSrc = new URL(`./${worker}`, import.meta.url).href;
  const doc = await pdfjs.getDocument({ url, httpHeaders: headers, isEvalSupported: false, enableXfa: false }).promise;
  into.replaceChildren();
  for (let n = 1; n <= Math.min(doc.numPages, 50); n++) {
    const page = await doc.getPage(n);
    const viewport = page.getViewport({ scale: 1.4 });
    const canvas = document.createElement("canvas");
    canvas.width = viewport.width;
    canvas.height = viewport.height;
    canvas.setAttribute("aria-label", `${n}`);
    into.append(canvas);
    await page.render({ canvasContext: canvas.getContext("2d")!, viewport }).promise;
  }
}

export { drawPdf };

@customElement("ortus-drive-file")
export class OrtusDriveFile extends LitElement {
  static override styles = [
    controls,
    css`
      :host {
        display: grid;
        grid-template-rows: auto 1fr;
        min-block-size: var(--ortus-screen, 100vh);
        color: var(--ortus-text);
      }
      header {
        display: flex;
        align-items: center;
        gap: 14px;
        padding-block: 14px;
        padding-inline: 24px;
        border-block-end: 1px solid var(--ortus-line);
      }
      .back {
        inline-size: 36px;
        block-size: 36px;
        border: 0;
        background: none;
        color: var(--ortus-text);
        font-size: 18px;
        cursor: pointer;
      }
      .badge {
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
      h1 {
        margin: 0;
        font-size: 16px;
      }
      header small {
        display: block;
        color: var(--ortus-text-3);
        font-size: 13px;
      }
      .spacer {
        flex: 1;
      }
      main {
        display: grid;
        justify-items: center;
        align-content: start;
        padding: 32px;
        overflow: auto;
      }
      .pages {
        display: grid;
        gap: 16px;
        justify-items: center;
      }
      .pages canvas,
      .picture {
        max-inline-size: min(100%, 900px);
        block-size: auto;
        border-radius: 6px;
        background: #fff;
      }
      .text {
        inline-size: min(100%, 760px);
        min-block-size: 60vh;
        padding: 48px;
        box-sizing: border-box;
        border-radius: 6px;
        background: #f4f5f8;
        color: #1b1d24;
        white-space: pre-wrap;
        line-height: 1.6;
        font-size: 15px;
      }
      .none {
        display: grid;
        gap: 14px;
        justify-items: center;
        padding: 64px;
        color: var(--ortus-text-2);
      }
      dialog {
        inline-size: min(560px, 94vw);
        max-inline-size: calc(100vw - 16px);
        box-sizing: border-box;
        padding: 0;
        border-radius: 22px;
        border: 1px solid var(--ortus-line-strong);
        background: var(--ortus-surface);
        color: var(--ortus-text);
      }
      dialog::backdrop {
        background: rgb(0 0 0 / 0.55);
      }
      form {
        display: grid;
        gap: 16px;
        padding: 28px;
      }
      form h2 {
        margin: 0;
        font-size: 20px;
      }
      .add {
        display: flex;
        gap: 10px;
      }
      .add input {
        flex: 1;
      }
      ul {
        margin: 0;
        padding: 0;
        list-style: none;
        display: grid;
        gap: 12px;
      }
      li.person {
        display: flex;
        align-items: center;
        gap: 14px;
      }
      li.person > span:not(.avatar) {
        flex: 1;
        min-inline-size: 0;
      }
      form > * {
        min-inline-size: 0;
      }
      .add input {
        min-inline-size: 0;
      }
      .add select {
        inline-size: 140px;
        flex: none;
      }
      label.field {
        display: grid;
        gap: 6px;
        font-size: 13px;
        color: var(--ortus-text-2);
      }
      li.person small {
        display: block;
        color: var(--ortus-text-3);
      }
      .avatar {
        flex: none;
        display: grid;
        place-items: center;
        inline-size: 40px;
        block-size: 40px;
        border-radius: 12px;
        background: #a5b4ff;
        color: #0a0b10;
        font-weight: 700;
        font-size: 14px;
      }
      .linkbox {
        display: grid;
        gap: 14px;
        padding: 20px;
        border-radius: 16px;
        border: 1px solid var(--ortus-line);
      }
      .linkbox .head {
        display: flex;
        justify-content: space-between;
        align-items: center;
      }
      .anyone {
        padding-block: 2px;
        padding-inline: 8px;
        border-radius: 6px;
        background: color-mix(in srgb, var(--ortus-orange) 18%, transparent);
        color: var(--ortus-orange);
        font-size: 13px;
      }
      .url {
        display: flex;
        gap: 8px;
        align-items: center;
        padding: 6px;
        padding-inline-start: 14px;
        border-radius: 12px;
        border: 1px solid var(--ortus-line-strong);
        font-family: var(--ortus-mono, monospace);
        color: var(--ortus-accent);
        font-size: 13px;
      }
      .url span {
        flex: 1;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      .options {
        display: grid;
        grid-template-columns: repeat(3, 1fr);
        gap: 10px;
      }
      .options label {
        display: grid;
        gap: 4px;
        padding: 12px;
        border-radius: 12px;
        border: 1px solid var(--ortus-line);
        font-size: 13px;
        color: var(--ortus-text-3);
      }
      .options input {
        font: inherit;
        color: var(--ortus-text);
        background: transparent;
        border: 0;
        padding: 0;
      }
      .foot {
        display: flex;
        align-items: center;
        justify-content: space-between;
      }
      .foot small {
        color: var(--ortus-text-3);
      }
      .error {
        margin: 0;
        color: var(--ortus-red);
        font-size: 14px;
      }
      .check {
        display: flex;
        gap: 10px;
        align-items: center;
        font-size: 14px;
      }
      .check input {
        flex: none;
        inline-size: 18px;
        block-size: 18px;
        margin: 0;
      }
      .url span {
        min-inline-size: 0;
      }
      .linkbox > *,
      .url,
      .options {
        min-inline-size: 0;
      }
      .options {
        grid-template-columns: repeat(3, minmax(0, 1fr));
      }
      select.input {
        block-size: 44px;
        padding-inline: 12px;
        border-radius: 12px;
        border: 1px solid var(--ortus-line-strong);
        background: var(--ortus-surface-2);
        color: var(--ortus-text);
        font: inherit;
        font-size: 14px;
      }
      .options label {
        min-inline-size: 0;
      }
      .options input {
        min-inline-size: 0;
        inline-size: 100%;
      }
      /* A phone: the header's buttons wrap under the name, the page fills
         the width, the share dialog stacks. */
      @media (max-width: 720px) {
        :host {
          grid-template-columns: minmax(0, 1fr);
        }
        header {
          flex-wrap: wrap;
          gap: 8px 10px;
          padding-block: 10px;
          padding-inline: 8px 12px;
        }
        header > div {
          flex: 1;
          min-inline-size: 0;
        }
        h1 {
          overflow: hidden;
          text-overflow: ellipsis;
          white-space: nowrap;
        }
        .spacer {
          flex-basis: 100%;
          block-size: 0;
        }
        .back {
          inline-size: 44px;
          block-size: 44px;
        }
        .button {
          min-block-size: 44px;
        }
        main {
          padding: 12px;
        }
        .text {
          padding: 20px;
        }
        .none {
          padding: 24px;
        }
        form {
          padding: 16px;
        }
        .add {
          flex-wrap: wrap;
        }
        .add input {
          flex-basis: 100%;
        }
        .add select {
          flex: 1;
        }
        .options {
          grid-template-columns: minmax(0, 1fr);
        }
        li.person {
          gap: 10px;
        }
        li.person small {
          overflow-wrap: anywhere;
        }
        .url {
          padding-inline-start: 10px;
        }
      }
    `,
  ];

  private readonly localize = new LocalizeController(this);
  @state() private item: Item | null = null;
  @state() private text: string | null = null;
  @state() private shares: Shares | null = null;
  @state() private versionList: Version[] = [];
  @state() private error = "";
  @state() private reason = "";
  @state() private needsReason = false;
  @state() private shareRole = "edit";
  @state() private labels: { id: string; name: string }[] = [];

  private fileId(): string {
    return new URLSearchParams(location.search).get("file") ?? "";
  }

  override connectedCallback(): void {
    super.connectedCallback();
    void this.load();
  }

  private async load(): Promise<void> {
    try {
      this.item = await call<Item>("GET", `/drive/items/${encodeURIComponent(this.fileId())}`);
      await this.updateComplete;
      await this.preview();
      if (new URLSearchParams(location.search).get("share") === "1") void this.openShare();
    } catch (e) {
      this.error = e instanceof ApiError ? e.message : String(e);
    }
  }

  private async preview(): Promise<void> {
    const item = this.item;
    if (!item || item.kind === "folder") return;
    if (item.mime === "application/pdf" || item.name.toLowerCase().endsWith(".pdf")) {
      const into = this.renderRoot.querySelector<HTMLElement>(".pages");
      if (into) await drawPdf(`/api/v1/drive/items/${item.id}/content?inline=1`, into).catch(() => (this.text = null));
      return;
    }
    if (!item.mime.startsWith("image/")) {
      this.text = (await call<{ text: string | null }>("GET", `/drive/items/${item.id}/text`).catch(() => ({ text: null }))).text;
    }
  }

  private async openShare(): Promise<void> {
    this.error = "";
    this.needsReason = false;
    this.shares = await call<Shares>("GET", `/drive/items/${this.item!.id}/shares`);
    if (this.item!.role === "owner" && !this.labels.length) this.labels = (await call<{ labels: { id: string; name: string }[] }>("GET", "/drive/labels").catch(() => ({ labels: [] }))).labels;
    await this.updateComplete;
    this.renderRoot.querySelector<HTMLDialogElement>("dialog.share")?.showModal();
  }

  private async addPerson(e: Event): Promise<void> {
    e.preventDefault();
    const input = this.renderRoot.querySelector<HTMLInputElement>("input.who")!;
    const who = input.value.trim();
    if (!who) return;
    try {
      this.shares = await call<Shares>("POST", `/drive/items/${this.item!.id}/shares`, { email: who, role: this.shareRole, reason: this.reason || undefined });
      input.value = "";
      this.needsReason = false;
      this.reason = "";
      this.error = "";
    } catch (err) {
      if (err instanceof ApiError && err.code === "needs_reason") this.needsReason = true;
      this.error = err instanceof ApiError ? err.message : String(err);
    }
  }

  private async makeLink(options: { expiresAt?: number | null; password?: string; noDownload?: boolean }): Promise<void> {
    try {
      const link = this.shares?.link;
      if (link) {
        // The link stays; only its settings change.
        this.shares = await call<Shares>("PATCH", `/drive/shares/${link.share}`, { expiresAt: options.expiresAt === undefined ? undefined : (options.expiresAt ?? 0), password: options.password, noDownload: options.noDownload });
        this.error = "";
        return;
      }
      this.shares = await call<Shares>("POST", `/drive/items/${this.item!.id}/link`, { expiresAt: options.expiresAt ?? this.shares?.link?.expiresAt ?? null, password: options.password, noDownload: options.noDownload ?? this.shares?.link?.noDownload ?? false, reason: this.reason || undefined });
      this.error = "";
    } catch (err) {
      if (err instanceof ApiError && err.code === "needs_reason") this.needsReason = true;
      this.error = err instanceof ApiError ? err.message : String(err);
    }
  }

  private async setLabel(label: string): Promise<void> {
    try {
      const r = await call<Shares & { removed: number }>("PUT", `/drive/items/${this.item!.id}/label`, { label: label || null });
      this.shares = r;
      this.error = r.removed ? t(this.localize, "sharesRemoved", { count: r.removed }) : "";
    } catch (err) {
      this.error = err instanceof ApiError ? err.message : String(err);
    }
  }

  private async unshare(share: string): Promise<void> {
    try {
      this.shares = await call<Shares>("DELETE", `/drive/shares/${share}`);
    } catch (err) {
      this.error = err instanceof ApiError ? err.message : String(err);
    }
  }

  private async openVersions(): Promise<void> {
    this.versionList = (await call<{ versions: Version[] }>("GET", `/drive/items/${this.item!.id}/versions`)).versions;
    await this.updateComplete;
    this.renderRoot.querySelector<HTMLDialogElement>("dialog.versions")?.showModal();
  }

  private async restoreVersion(v: number): Promise<void> {
    try {
      this.item = await call<Item>("POST", `/drive/items/${this.item!.id}/versions/${v}/restore`);
      this.renderRoot.querySelector<HTMLDialogElement>("dialog.versions")?.close();
      await this.preview();
    } catch (err) {
      this.error = err instanceof ApiError ? err.message : String(err);
    }
  }

  private sendByEmail(): void {
    const item = this.item!;
    const link = `${location.origin}/drive?file=${encodeURIComponent(item.id)}`;
    go(`/mail/compose?subject=${encodeURIComponent(item.name)}&text=${encodeURIComponent(link)}`);
  }

  override render() {
    const item = this.item;
    if (!item) return html`${this.error ? html`<p class="error" role="alert">${this.error}</p>` : nothing}`;
    const canShare = item.role === "owner" || item.role === "edit";
    const isPdf = item.mime === "application/pdf" || item.name.toLowerCase().endsWith(".pdf");
    return html`<header>
        <button class="back" aria-label=${t(this.localize, "back")} @click=${() => go(item.parent ? `/drive?folder=${encodeURIComponent(item.parent)}` : "/drive")}>${document.dir === "rtl" ? "›" : "‹"}</button>
        <span class="badge" aria-hidden="true">${badge(item)}</span>
        <div><h1 dir="auto">${item.name}</h1><small>${item.kind === "file" ? `${sizeText(item.size)} · ` : ""}${whenText(item.modifiedAt)}</small></div>
        <span class="spacer"></span>
        ${item.kind === "file" ? html`<button class="button" @click=${this.openVersions}>${t(this.localize, "versions")}</button><button class="button" @click=${this.sendByEmail}>${t(this.localize, "sendByEmail")}</button>` : nothing}
        ${item.kind === "file" ? html`<a class="button" href="/api/v1/drive/items/${item.id}/content" download=${item.name}>${t(this.localize, "download")}</a>` : nothing}
        ${canShare ? html`<button class="button primary" @click=${this.openShare}>${t(this.localize, "share")}</button>` : nothing}
      </header>
      <main>
        ${item.kind === "folder"
          ? html`<p class="none">${t(this.localize, "folderShareHint")}</p>`
          : item.mime.startsWith("image/")
            ? html`<img class="picture" src="/api/v1/drive/items/${item.id}/content?inline=1" alt=${item.name} />`
            : isPdf
              ? html`<div class="pages" aria-label=${item.name}></div>`
              : this.text
                ? html`<article class="text" dir="auto">${this.text}</article>`
                : html`<div class="none"><p>${t(this.localize, "noPreview")}</p><a class="button primary" href="/api/v1/drive/items/${item.id}/content" download=${item.name}>${t(this.localize, "download")}</a></div>`}
      </main>
      ${this.renderShare(item)} ${this.renderVersions()}`;
  }

  private renderShare(item: Item) {
    const s = this.shares;
    const roles: [string, Parameters<typeof t>[1]][] = [
      ["view", "canView"],
      ["comment", "canComment"],
      ["edit", "canEdit"],
    ];
    const day = (ms: number | null) => (ms ? new Date(ms).toISOString().slice(0, 10) : "");
    return html`<dialog class="share" aria-label=${t(this.localize, "shareNamed", { name: item.name })}>
      ${s
        ? html`<form method="dialog" @submit=${(e: SubmitEvent) => e.submitter && (e.submitter as HTMLButtonElement).value !== "done" && e.preventDefault()}>
            <h2>${t(this.localize, "shareNamed", { name: item.name })}</h2>
            <div class="add">
              <input class="input who" type="email" placeholder=${t(this.localize, "addPeople")} aria-label=${t(this.localize, "addPeople")} @keydown=${(e: KeyboardEvent) => e.key === "Enter" && this.addPerson(e)} />
              <select class="input" aria-label=${t(this.localize, "role")} .value=${this.shareRole} @change=${(e: Event) => (this.shareRole = (e.target as HTMLSelectElement).value)}>
                ${roles.map(([r, key]) => html`<option value=${r} ?selected=${r === this.shareRole}>${t(this.localize, key)}</option>`)}
              </select>
              <button type="button" class="button" @click=${this.addPerson}>${t(this.localize, "add")}</button>
            </div>
            ${this.needsReason ? html`<label class="field">${t(this.localize, "reason")}<input class="input" .value=${this.reason} @input=${(e: Event) => (this.reason = (e.target as HTMLInputElement).value)} /></label>` : nothing}
            ${this.error ? html`<p class="error" role="alert">${this.error}</p>` : nothing}
            <ul aria-label=${t(this.localize, "whoHasAccess")}>
              ${s.people.map(
                (p) => html`<li class="person">
                  <span class="avatar" aria-hidden="true">${(p.name || p.email).slice(0, 2).toUpperCase()}</span>
                  <span>${p.you ? t(this.localize, "you") : p.name}<small>${p.email}</small></span>
                  ${p.role === "owner" ? html`<small>${t(this.localize, "ownerRole")}</small>` : html`<small>${t(this.localize, roles.find(([r]) => r === p.role)?.[1] ?? "canView")}</small>${p.share ? html`<button type="button" class="button small" aria-label=${t(this.localize, "removeAccess", { who: p.name })} @click=${() => this.unshare(p.share!)}>×</button>` : nothing}`}
                </li>`,
              )}
              ${s.groups.map((g) => html`<li class="person"><span class="avatar" aria-hidden="true">${g.name.slice(0, 2).toUpperCase()}</span><span>${g.name}</span><small>${t(this.localize, roles.find(([r]) => r === g.role)?.[1] ?? "canView")}</small><button type="button" class="button small" aria-label=${t(this.localize, "removeAccess", { who: g.name })} @click=${() => this.unshare(g.share)}>×</button></li>`)}
              ${s.outside.map((o) => html`<li class="person"><span class="avatar" aria-hidden="true">${o.email.slice(0, 2).toUpperCase()}</span><span>${o.email}<small>${t(this.localize, "personalLink")}</small></span><small>${t(this.localize, "canView")}</small><button type="button" class="button small" aria-label=${t(this.localize, "removeAccess", { who: o.email })} @click=${() => this.unshare(o.share)}>×</button></li>`)}
            </ul>
            <section class="linkbox" aria-label=${t(this.localize, "linkSharing")}>
              <div class="head"><strong>${t(this.localize, "linkSharing")}</strong>${s.link ? html`<span class="anyone">${t(this.localize, "anyoneWithLink")}</span>` : nothing}</div>
              ${s.link
                ? html`<div class="url"><span>${s.link.url.replace(/^https?:\/\//, "")}</span><button type="button" class="button small" @click=${() => navigator.clipboard?.writeText(s.link!.url)}>${t(this.localize, "copy")}</button></div>
                    <div class="options">
                      <label>${t(this.localize, "access")}<span>${t(this.localize, "viewOnly")}</span></label>
                      <label>${t(this.localize, "expiresLabel")}<input type="date" .value=${day(s.link.expiresAt)} @change=${(e: Event) => this.makeLink({ expiresAt: Date.parse(`${(e.target as HTMLInputElement).value}T23:59:59`) || null })} /></label>
                      <label>${t(this.localize, "password")}<input type="password" minlength="8" placeholder=${s.link.password ? t(this.localize, "on") : t(this.localize, "off")} @change=${(e: Event) => this.makeLink({ password: (e.target as HTMLInputElement).value })} /></label>
                    </div>
                    <label class="check"><input type="checkbox" .checked=${s.link.noDownload} @change=${(e: Event) => this.makeLink({ noDownload: (e.target as HTMLInputElement).checked })} />${t(this.localize, "blockDownloads")}</label>
                    <button type="button" class="button" @click=${() => this.unshare(s.link!.share)}>${t(this.localize, "stopLink")}</button>`
                : html`<button type="button" class="button" @click=${() => this.makeLink({})}>${t(this.localize, "makeLink")}</button>`}
            </section>
            ${item.role === "owner" && this.labels.length
              ? html`<label class="field">${t(this.localize, "label")}<select class="input" .value=${s.label ?? ""} @change=${(e: Event) => this.setLabel((e.target as HTMLSelectElement).value)}>
                  <option value="" ?selected=${!s.label}>${t(this.localize, "defaultLabel")}</option>
                  ${this.labels.map((l) => html`<option value=${l.id} ?selected=${l.id === s.label}>${l.name}</option>`)}
                </select></label>`
              : nothing}
            <div class="foot">
              <small>${s.link ? t(this.localize, "openedTimes", { count: s.link.opened }) : ""}${s.label ? ` · ${s.label.toUpperCase()}` : ""}</small>
              <button class="button primary" value="done">${t(this.localize, "done")}</button>
            </div>
          </form>`
        : nothing}
    </dialog>`;
  }

  private renderVersions() {
    return html`<dialog class="versions" aria-label=${t(this.localize, "versions")}>
      <form method="dialog">
        <h2>${t(this.localize, "versions")}</h2>
        <ul>
          ${this.versionList.map(
            (v, i) => html`<li class="person">
              <span>${t(this.localize, "versionN", { n: v.version })}<small>${whenText(v.createdAt)}${v.createdBy ? ` · ${v.createdBy}` : ""} · ${sizeText(v.size)}</small></span>
              ${i === 0 ? html`<small>${t(this.localize, "current")}</small>` : html`<button type="button" class="button small" @click=${() => this.restoreVersion(v.version)}>${t(this.localize, "restore")}</button>`}
            </li>`,
          )}
        </ul>
        <div class="foot"><span></span><button class="button primary">${t(this.localize, "done")}</button></div>
      </form>
    </dialog>`;
  }
}
