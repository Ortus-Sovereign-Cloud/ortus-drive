// The drive card on the start page (ortus-drive-today, from
// /assets/apps/drive/today.js): the files one worked on last.
import { LitElement, LocalizeController, call, css, customElement, html, nothing, state } from "ortus-kit";
import type { Item } from "./drive.js";
import { ready, t } from "./i18n.js";

await ready;

@customElement("ortus-drive-today")
export class OrtusDriveToday extends LitElement {
  static override styles = css`
    :host {
      display: block;
    }
    section {
      padding: 20px;
      border-radius: 18px;
      border: 1px solid var(--ortus-line);
      background: var(--ortus-surface);
    }
    h2 {
      margin: 0 0 12px;
      font-size: 17px;
      font-weight: 600;
    }
    ul {
      margin: 0;
      padding: 0;
      list-style: none;
    }
    li a {
      display: flex;
      flex-direction: column;
      padding: 8px;
      margin-inline: -8px;
      border-radius: 10px;
      color: inherit;
      text-decoration: none;
    }
    li a:hover,
    li a:focus-visible {
      background: var(--ortus-hover);
    }
    strong {
      font-size: 14px;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }
    small {
      color: var(--ortus-text-2);
      font-size: 13px;
    }
    .quiet {
      margin: 0;
      color: var(--ortus-text-2);
      font-size: 14px;
    }
    footer {
      margin-block-start: 12px;
      font-size: 14px;
    }
    footer a {
      color: var(--ortus-accent);
    }
  `;

  private readonly localize = new LocalizeController(this);
  @state() private items: Item[] = [];
  @state() private loaded = false;

  override connectedCallback(): void {
    super.connectedCallback();
    void call<{ items: Item[] }>("GET", "/drive/special?which=recent")
      .then((r) => (this.items = r.items.filter((i) => i.kind === "file").slice(0, 4)))
      .catch(() => undefined)
      .finally(() => (this.loaded = true));
  }

  private href(item: Item): string {
    return item.mime === "application/vnd.ortus.doc" ? `/docs?doc=${encodeURIComponent(item.id)}` : `/drive?file=${encodeURIComponent(item.id)}`;
  }

  override render() {
    if (!this.loaded) return nothing;
    const when = new Intl.RelativeTimeFormat(this.localize.lang, { numeric: "auto" });
    const ago = (ms: number) => {
      const minutes = Math.round((Date.now() - ms) / 60_000);
      return minutes < 60 ? when.format(-minutes, "minute") : minutes < 1440 ? when.format(-Math.round(minutes / 60), "hour") : when.format(-Math.round(minutes / 1440), "day");
    };
    return html`
      <section aria-labelledby="drive-today">
        <h2 id="drive-today">${t(this.localize, "recent")}</h2>
        ${this.items.length
          ? html`<ul>
              ${this.items.map(
                (item) => html`<li>
                  <a href=${this.href(item)}><strong dir="auto">${item.name}</strong><small>${ago(item.modifiedAt)}</small></a>
                </li>`,
              )}
            </ul>`
          : html`<p class="quiet">${t(this.localize, "todayNoFiles")}</p>`}
        <footer><a href="/drive">${t(this.localize, "todayOpen")}</a></footer>
      </section>
    `;
  }
}

declare global {
  interface HTMLElementTagNameMap {
    "ortus-drive-today": OrtusDriveToday;
  }
}
