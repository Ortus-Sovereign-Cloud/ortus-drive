// What a shared link opens, for anyone (/drive/public/s/<token>): the
// password when it has one, then the file (viewed, and downloaded unless
// the link forbids it) or the folder to look through.
import { LitElement, LocalizeController, MARK, controls, css, customElement, html, nothing, state, unsafeSVG } from "ortus-kit";
import { badge, sizeText } from "./drive.js";
import { t } from "./i18n.js";
import { drawPdf } from "./viewer.js";

interface Entry {
  id: string;
  kind: string;
  name: string;
  size: number;
  mime: string;
}

interface Opened {
  needsPassword: boolean;
  item?: Entry;
  children?: Entry[];
  path?: { id: string; name: string }[];
  owner?: string;
  noDownload?: boolean;
}

@customElement("ortus-drive-public")
export class OrtusDrivePublic extends LitElement {
  static override styles = [
    controls,
    css`
      :host {
        display: block;
        min-block-size: 100dvh;
        padding: 24px;
        box-sizing: border-box;
        background: var(--ortus-bg);
        color: var(--ortus-text);
      }
      main {
        max-inline-size: 960px;
        margin-inline: auto;
        display: grid;
        gap: 20px;
      }
      .bar {
        display: flex;
        align-items: center;
        gap: 14px;
      }
      .bar h1 {
        margin: 0;
        font-size: 20px;
        flex: 1;
        overflow-wrap: anywhere;
      }
      .mark {
        inline-size: 32px;
        block-size: 32px;
      }
      p {
        margin: 0;
        color: var(--ortus-text-2);
      }
      form {
        display: grid;
        gap: 12px;
        max-inline-size: 360px;
      }
      ul {
        margin: 0;
        padding: 0;
        list-style: none;
        border-radius: 16px;
        border: 1px solid var(--ortus-line);
        background: var(--ortus-surface);
      }
      li button {
        display: flex;
        align-items: center;
        gap: 14px;
        inline-size: 100%;
        padding-block: 12px;
        padding-inline: 16px;
        border: 0;
        border-block-end: 1px solid var(--ortus-line);
        background: none;
        color: inherit;
        font: inherit;
        text-align: start;
        cursor: pointer;
      }
      .badge {
        display: grid;
        place-items: center;
        inline-size: 36px;
        block-size: 36px;
        border-radius: 10px;
        background: var(--ortus-surface-2);
        color: var(--ortus-accent);
        font-size: 11px;
        font-weight: 700;
      }
      .view {
        display: grid;
        justify-items: center;
        gap: 16px;
        position: relative;
      }
      .view img,
      .view canvas {
        max-inline-size: 100%;
        border-radius: 6px;
      }
      .text {
        inline-size: 100%;
        padding: 32px;
        box-sizing: border-box;
        border-radius: 6px;
        background: #f4f5f8;
        color: #1b1d24;
        white-space: pre-wrap;
      }
      .mark-over {
        position: absolute;
        inset: 0;
        display: grid;
        place-items: center;
        pointer-events: none;
        color: rgb(120 120 140 / 0.25);
        font-size: 36px;
        font-weight: 700;
        rotate: -24deg;
      }
      .error {
        color: var(--ortus-red);
      }
      .quiet {
        font-size: 14px;
        color: var(--ortus-text-3);
      }
      @media (max-width: 720px) {
        :host {
          padding: 12px;
        }
        .bar {
          flex-wrap: wrap;
        }
        .text {
          padding: 16px;
        }
      }
    `,
  ];

  private readonly localize = new LocalizeController(this);
  @state() private opened: Opened | null = null;
  @state() private password = "";
  @state() private problem = "";
  @state() private picture = "";
  @state() private text: string | null = null;
  @state() private at = "";

  private token(): string {
    return location.pathname.split("/").pop() ?? "";
  }

  private headers(): Record<string, string> {
    return this.password ? { "X-Ortus-Link-Password": this.password } : {};
  }

  override connectedCallback(): void {
    super.connectedCallback();
    void this.load();
  }

  private async load(): Promise<void> {
    this.problem = "";
    this.picture = "";
    this.text = null;
    const r = await fetch(`/api/v1/drive/links/${encodeURIComponent(this.token())}${this.at ? `?item=${encodeURIComponent(this.at)}` : ""}`, { headers: this.headers() });
    if (!r.ok) {
      this.problem = r.status === 401 ? t(this.localize, "wrongPassword") : r.status === 429 ? t(this.localize, "tooManyTries") : t(this.localize, "linkEnded");
      if (r.status !== 401 && r.status !== 429) this.opened = null;
      return;
    }
    this.opened = (await r.json()) as Opened;
    await this.updateComplete;
    await this.preview();
  }

  private content(inline: boolean): string {
    const item = this.opened?.item;
    const at = this.at && item ? `item=${encodeURIComponent(item.id)}&` : "";
    return `/api/v1/drive/links/${encodeURIComponent(this.token())}/content?${at}${inline ? "inline=1" : ""}`;
  }

  private async preview(): Promise<void> {
    const item = this.opened?.item;
    if (!item || item.kind === "folder") return;
    if (item.mime.startsWith("image/")) {
      const r = await fetch(this.content(true), { headers: this.headers() });
      if (!r.ok) return;
      // A data: address: the page's policy allows no other kind.
      const blob = await r.blob();
      this.picture = await new Promise<string>((done) => {
        const reader = new FileReader();
        reader.onload = () => done(String(reader.result));
        reader.readAsDataURL(blob);
      });
    } else if (item.mime === "application/pdf") {
      const into = this.renderRoot.querySelector<HTMLElement>(".pages");
      if (into) await drawPdf(this.content(true), into, this.headers()).catch(() => undefined);
    } else if (item.mime === "text/plain") {
      const r = await fetch(this.content(true), { headers: this.headers() });
      if (r.ok) this.text = await r.text();
    }
  }

  private async download(): Promise<void> {
    const item = this.opened?.item;
    if (!item) return;
    const r = await fetch(this.content(false), { headers: this.headers() });
    if (!r.ok) return;
    const url = URL.createObjectURL(await r.blob());
    const a = document.createElement("a");
    a.href = url;
    a.download = item.name;
    a.click();
    window.setTimeout(() => URL.revokeObjectURL(url), 10_000);
  }

  override render() {
    const o = this.opened;
    return html`<main>
      <div class="bar">
        <span class="mark">${unsafeSVG(MARK)}</span>
        <h1 dir="auto">${o?.item?.name ?? t(this.localize, "sharedFile")}</h1>
        ${o?.item && o.item.kind === "file" && !o.noDownload ? html`<button class="button primary" @click=${this.download}>${t(this.localize, "download")}</button>` : nothing}
      </div>
      ${o?.owner ? html`<p>${t(this.localize, "sharedBy", { who: o.owner })}${o.item?.kind === "file" ? ` · ${sizeText(o.item.size)}` : ""}</p>` : nothing}
      ${this.problem ? html`<p class="error" role="alert">${this.problem}</p>` : nothing}
      ${o?.needsPassword || this.problem === t(this.localize, "wrongPassword")
        ? html`<form @submit=${(e: Event) => (e.preventDefault(), this.load())}>
            <label>${t(this.localize, "linkPassword")}<input class="input" type="password" required .value=${this.password} @input=${(e: Event) => (this.password = (e.target as HTMLInputElement).value)} /></label>
            <button class="button primary">${t(this.localize, "openLink")}</button>
          </form>`
        : nothing}
      ${o?.item?.kind === "folder"
        ? html`${this.at ? html`<button class="button" @click=${() => ((this.at = ""), this.load())}>${t(this.localize, "back")}</button>` : nothing}
            <ul aria-label=${o.item.name}>
              ${(o.children ?? []).map((c) => html`<li><button @click=${() => ((this.at = c.id), this.load())}><span class="badge" aria-hidden="true">${badge(c)}</span><span dir="auto">${c.name}</span></button></li>`)}
            </ul>`
        : o?.item
          ? html`<section class="view" aria-label=${o.item.name}>
              ${this.picture ? html`<img src=${this.picture} alt=${o.item.name} />` : nothing}
              ${o.item.mime === "application/pdf" ? html`<div class="pages"></div>` : nothing}
              ${this.text !== null ? html`<article class="text" dir="auto">${this.text}</article>` : nothing}
              ${o.noDownload ? html`<span class="mark-over" aria-hidden="true">${t(this.localize, "viewOnly")}</span>` : nothing}
              ${!this.picture && this.text === null && o.item.mime !== "application/pdf" ? html`<p class="quiet">${t(this.localize, "noPreview")}</p>` : nothing}
            </section>`
          : nothing}
      <p class="quiet">${t(this.localize, "noAccountNeeded")}</p>
    </main>`;
  }
}
