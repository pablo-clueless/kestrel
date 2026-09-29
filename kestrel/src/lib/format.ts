/** Milliseconds: two decimals below 10, one above. */
export const ms = (v: number | null | undefined) =>
  v == null ? "–" : v < 10 ? v.toFixed(2) : v.toFixed(1);

/** A whole number with thousands separators. */
export const int = (v: number | null | undefined) =>
  v == null ? "–" : Math.round(v).toLocaleString();
