export { cn } from "cn";

export type ExportFormat = "json" | "yaml" | "auto";

interface DownloadOptions {
  /** With or without an extension; `auto` picks YAML for `.yaml`/`.yml` names, JSON otherwise. */
  fileName: string;
  format?: ExportFormat;
}

/** Saves `text` as a file named `fileName`, through a temporary link. */
export function downloadText(text: string, fileName: string, type = "text/plain") {
  const url = URL.createObjectURL(new Blob([text], { type }));
  const link = Object.assign(document.createElement("a"), { href: url, download: fileName });
  document.body.appendChild(link);
  link.click();
  link.remove();
  // Revoking straight after click() can cancel the download in Firefox and Safari.
  setTimeout(() => URL.revokeObjectURL(url), 10_000);
}

/**
 * Saves `content` as a pretty-printed JSON or YAML file. A string is parsed first (JSON, then
 * YAML); it must hold an object or array, or this rejects with a message fit for a toast.
 *
 * Async so callers can show a loading state: js-yaml is only fetched when a download happens,
 * keeping it out of every bundle that imports `cn` from here.
 */
export async function downloadJsonOrYaml(
  content: string | Record<string, unknown> | unknown[],
  { fileName, format = "auto" }: DownloadOptions,
): Promise<void> {
  const { dump, load } = await import("js-yaml");

  let parsed: unknown = content;
  if (typeof content === "string") {
    const trimmed = content.trim();
    try {
      parsed = JSON.parse(trimmed);
    } catch {
      try {
        parsed = load(trimmed);
      } catch {
        parsed = undefined;
      }
    }
    // YAML reads almost any text as a plain string, so only structured data counts.
    if (parsed === null || typeof parsed !== "object") {
      throw new Error("The content isn't valid JSON or YAML.");
    }
  }

  const resolved = format !== "auto" ? format : /\.(yaml|yml)$/i.test(fileName) ? "yaml" : "json";
  const text = resolved === "yaml" ? dump(parsed, { indent: 2 }) : JSON.stringify(parsed, null, 2);
  const hasExtension = (resolved === "yaml" ? /\.(yaml|yml)$/i : /\.json$/i).test(fileName);
  const name = hasExtension ? fileName : `${fileName}.${resolved}`;
  const type = resolved === "yaml" ? "application/yaml" : "application/json";
  downloadText(text, name, type);
}

/** A random (v4) UUID. `crypto.randomUUID` only exists on secure origins (https, or localhost), so
 * on plain http (e.g. a LAN address) this builds one from `crypto.getRandomValues`, which works
 * everywhere and, unlike `Math.random`, is safe for ids that act as keys. */
export function generateUUID(): string {
  if (typeof crypto.randomUUID === "function") return crypto.randomUUID();
  const b = crypto.getRandomValues(new Uint8Array(16));
  b[6] = (b[6] & 0x0f) | 0x40; // version 4
  b[8] = (b[8] & 0x3f) | 0x80; // RFC 4122 variant
  const h = Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");
  return `${h.slice(0, 8)}-${h.slice(8, 12)}-${h.slice(12, 16)}-${h.slice(16, 20)}-${h.slice(20)}`;
}

/** Whether `s` is a UUID in the usual 8-4-4-4-12 form. */
export const isUUID = (s: string) =>
  /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(s);
