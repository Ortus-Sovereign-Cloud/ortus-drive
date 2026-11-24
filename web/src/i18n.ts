// The drive app's texts, in the languages of Ortus, through the core's kit.
import {
  appText,
  registerAppMessages,
  type LocalizeController,
  type Message,
} from "ortus-kit";
import en, { type DriveKey } from "./locales/en.js";

const catalogs: Record<
  string,
  () => Promise<{ default: Record<string, Message> }>
> = {
  pl: () => import("./locales/pl.js"),
  uk: () => import("./locales/uk.js"),
  de: () => import("./locales/de.js"),
  fr: () => import("./locales/fr.js"),
  es: () => import("./locales/es.js"),
  it: () => import("./locales/it.js"),
  ar: () => import("./locales/ar.js"),
};

export const ready = registerAppMessages("drive", en, async (code) =>
  code === "en" ? en : ((await catalogs[code]?.())?.default ?? {}),
);

export function t(
  localize: LocalizeController,
  key: DriveKey,
  args?: Record<string, string | number>,
): string {
  return appText(localize, "drive", key, args);
}
