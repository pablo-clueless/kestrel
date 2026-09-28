import type { ExtractSource } from "@/types/engine/ExtractSource";
import type { Sample } from "@/types/engine/Sample";

/** What the engine puts in place of secret values (engine/src/redact.rs). */
export const REDACTED = "[redacted]";

/** `data.items[0].id` → ["data", "items", 0, "id"]. Mirrors engine/src/extract.rs; null if malformed. */
const parsePath = (path: string): (string | number)[] | null => {
  const p = path.trim().replace(/^\$\.?/, "");
  const steps: (string | number)[] = [];
  for (const segment of p.split(".")) {
    const m = segment.match(/^([^[\]]*)((?:\[\s*\d+\s*\])*)$/);
    if (!m || (!m[1] && !m[2])) return null;
    if (m[1]) steps.push(m[1]);
    for (const [, i] of m[2].matchAll(/\[\s*(\d+)\s*\]/g)) steps.push(Number(i));
  }
  return steps;
};

/** The value a rule would save from `sample`, as the engine would render it. Previews only: the
 * sample's body is redacted and truncated, so the engine may see more. */
export const previewPick = (
  sample: Sample,
  source: ExtractSource,
  path: string,
): { value: string } | { error: string } => {
  if (source === "status") {
    return sample.status === null ? { error: "No response" } : { value: String(sample.status) };
  }
  if (source === "header") {
    const name = path.trim().toLowerCase();
    const hit = sample.responseHeaders.find(([k]) => k.toLowerCase() === name);
    return hit ? { value: hit[1] } : { error: `No \`${path.trim()}\` header in the response` };
  }
  if (!path.trim()) return { value: sample.body };
  let node: unknown;
  try {
    node = JSON.parse(sample.body);
  } catch {
    return { error: "The response body isn't JSON" };
  }
  const steps = parsePath(path);
  if (!steps) return { error: `Can't read path \`${path}\`` };
  for (const step of steps) {
    // Like serde_json: keys only index objects, numbers only index arrays.
    const fits = Array.isArray(node) ? typeof step === "number" : typeof step === "string";
    if (node === null || typeof node !== "object" || !fits || !(step in node)) {
      return { error: `\`${path}\` not found` };
    }
    node = (node as Record<string | number, unknown>)[step];
  }
  if (node === null) return { error: `\`${path}\` is null` };
  return { value: typeof node === "string" ? node : JSON.stringify(node) };
};

/** Paths to the scalar values in a JSON body, for path suggestions. Capped for huge responses. */
export const leafPaths = (body: string, limit = 200): string[] => {
  let root: unknown;
  try {
    root = JSON.parse(body);
  } catch {
    return [];
  }
  const out: string[] = [];
  const walk = (node: unknown, path: string) => {
    if (out.length >= limit) return;
    if (Array.isArray(node)) {
      node.forEach((v, i) => walk(v, `${path}[${i}]`));
    } else if (node !== null && typeof node === "object") {
      for (const [k, v] of Object.entries(node)) walk(v, path ? `${path}.${k}` : k);
    } else if (path && node !== null) {
      out.push(path);
    }
  };
  walk(root, "");
  return out;
};

/** A variable name from a path's last key: `data.access-token` → `accessToken`, `X-Request-Id` → `xRequestId`. */
export const suggestName = (source: ExtractSource, path: string) => {
  if (source === "status") return "status";
  const last =
    path
      .split(/[.[\]]/)
      .filter((s) => s && !/^\d+$/.test(s))
      .pop() ?? "";
  const camel = last.replace(/[^A-Za-z0-9_]+(.)?/g, (_, c: string | undefined) =>
    c ? c.toUpperCase() : "",
  );
  return camel.charAt(0).toLowerCase() + camel.slice(1);
};
