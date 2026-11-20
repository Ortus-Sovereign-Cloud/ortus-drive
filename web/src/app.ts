// The drive's client: the element the shell shows at /drive (the drive or
// an opened file, by address) and the page a shared link opens.
import { LitElement, css, customElement, html, state } from "ortus-kit";
import "./drive.js";
import "./public.js";
import "./viewer.js";
import { ready } from "./i18n.js";

await ready;

@customElement("ortus-drive-app")
export class OrtusDriveApp extends LitElement {
  static override styles = css`
    :host {
      display: block;
    }
  `;

  @state() private file = new URLSearchParams(location.search).get("file");

  private readonly onPop = (): void => {
    this.file = new URLSearchParams(location.search).get("file");
  };

  override connectedCallback(): void {
    super.connectedCallback();
    window.addEventListener("popstate", this.onPop);
  }

  override disconnectedCallback(): void {
    super.disconnectedCallback();
    window.removeEventListener("popstate", this.onPop);
  }

  override render() {
    // A new element each time: each loads what the address names.
    return this.file ? html`${[this.file].map(() => html`<ortus-drive-file></ortus-drive-file>`)}` : html`<ortus-drive-home></ortus-drive-home>`;
  }
}
