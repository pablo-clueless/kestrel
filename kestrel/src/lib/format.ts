/** Milliseconds: two decimals below 10, one above. */
export const ms = (v: number | null | undefined) =>
  v == null ? "–" : v < 10 ? v.toFixed(2) : v.toFixed(1);

/** A whole number with thousands separators. */
export const int = (v: number | null | undefined) =>
  v == null ? "–" : Math.round(v).toLocaleString();

const relativeFormat = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });

/** "5 minutes ago", "yesterday". */
export const relative = (ms: number) => {
  const seconds = Math.round((ms - Date.now()) / 1000);
  const units: [Intl.RelativeTimeFormatUnit, number][] = [
    ["year", 31_536_000],
    ["month", 2_592_000],
    ["day", 86_400],
    ["hour", 3_600],
    ["minute", 60],
  ];
  for (const [unit, size] of units) {
    if (Math.abs(seconds) >= size) return relativeFormat.format(Math.round(seconds / size), unit);
  }
  return "just now";
};

/** "12 Oct 2026". */
export const date = (ms: number) =>
  new Date(ms).toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" });

/** "1.2 MB": powers of 1024, one decimal from KB up. */
export const bytes = (n: number) => {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  while (n >= 1024 && i < units.length - 1) {
    n /= 1024;
    i++;
  }
  return i === 0 ? `${n} B` : `${n.toFixed(1)} ${units[i]}`;
};
