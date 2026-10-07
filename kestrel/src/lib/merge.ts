import type { Collection } from "@/types/engine/Collection";
import type { Environment } from "@/types/engine/Environment";
import type { Workspace } from "@/types/engine/Workspace";
import type { Endpoint } from "@/types/engine/Endpoint";

/**
 * Three-way merge of a workspace, for shared workspaces: `base` is what this tab last synced with
 * the engine, `mine` is what's on screen now, `theirs` is what the engine has now (someone else
 * saved meanwhile).
 *
 * Changes on one side only are taken as they are. Where both sides changed the same thing, the merge
 * goes down a level (collections → their fields, endpoints and variables; environments → their
 * variables) so that two people editing different requests in one collection don't collide. Only the
 * same endpoint, field or variable changed differently on both sides is a real conflict: this tab's
 * version wins (it's the one on screen) and it's named in `conflicts`. An item deleted on one side
 * and edited on the other is kept, edits included, since dropping it would lose work.
 *
 * The active collection and environment are this tab's view, so they stay as they are here (if they
 * still exist).
 */
export const mergeWorkspace = (
  base: Workspace,
  mine: Workspace,
  theirs: Workspace,
): { merged: Workspace; conflicts: string[] } => {
  const conflicts: string[] = [];
  const collections = mergeList(
    base.collections,
    mine.collections,
    theirs.collections,
    (c) => c.id,
    (b, m, t) => mergeCollection(b, m, t, conflicts),
    (c) => `collection "${c.name}"`,
    conflicts,
  );
  const environments = mergeList(
    base.environments,
    mine.environments,
    theirs.environments,
    (e) => e.name,
    (b, m, t) => mergeEnvironment(b, m, t, conflicts),
    (e) => `environment "${e.name}"`,
    conflicts,
  );
  const activeCollection = collections.some((c) => c.id === mine.activeCollection)
    ? mine.activeCollection
    : (collections[0]?.id ?? null);
  const activeEnvironment = environments.some((e) => e.name === mine.activeEnvironment)
    ? mine.activeEnvironment
    : null;
  return { merged: { collections, environments, activeCollection, activeEnvironment }, conflicts };
};

const mergeCollection = (
  b: Collection | undefined,
  m: Collection,
  t: Collection,
  conflicts: string[],
): Collection => {
  const base = b ?? ({ ...m, endpoints: [], vars: {}, groups: [] } as Collection);
  const label = `collection "${m.name}"`;
  const endpoints = mergeList(
    base.endpoints,
    m.endpoints,
    t.endpoints,
    (e) => e.id,
    // Two people changing one request: the request is the unit, this tab's wins.
    (_, mine) => {
      conflicts.push(`request "${endpointLabel(mine)}"`);
      return mine;
    },
    (e) => `request "${endpointLabel(e)}"`,
    conflicts,
  );
  const vars = mergeMap(base.vars, m.vars, t.vars, (k) => `variable "${k}" in ${label}`, conflicts);
  const groups = mergeSet(base.groups, m.groups, t.groups);
  return {
    ...mergeFields(base, m, t, ["endpoints", "vars", "groups"], label, conflicts),
    endpoints,
    vars,
    groups,
  };
};

const mergeEnvironment = (
  b: Environment | undefined,
  m: Environment,
  t: Environment,
  conflicts: string[],
): Environment => ({
  name: m.name,
  vars: mergeMap(
    b?.vars ?? {},
    m.vars,
    t.vars,
    (k) => `variable "${k}" in environment "${m.name}"`,
    conflicts,
  ),
});

const endpointLabel = (e: Endpoint) => e.name || `${e.method} ${e.url}` || "untitled";

/** Keyed list merge. `both` resolves an item changed on both sides (`base` undefined when both
 * added it). Order: theirs, unless this tab reordered; items only this tab has go at the end. */
const mergeList = <T>(
  base: T[],
  mine: T[],
  theirs: T[],
  key: (item: T) => string,
  both: (base: T | undefined, mine: T, theirs: T) => T,
  label: (item: T) => string,
  conflicts: string[],
): T[] => {
  const index = (items: T[]) => new Map(items.map((item) => [key(item), item]));
  const [b, m, t] = [index(base), index(mine), index(theirs)];
  const result = new Map<string, T>();
  for (const k of new Set([...m.keys(), ...t.keys(), ...b.keys()])) {
    const [bi, mi, ti] = [b.get(k), m.get(k), t.get(k)];
    let out: T | undefined;
    if (same(mi, bi)) out = ti;
    else if (same(ti, bi) || same(mi, ti)) out = mi;
    else if (mi === undefined) {
      // Deleted here, edited there: keep their edits.
      out = ti;
      conflicts.push(`${label(ti!)} (you deleted it, but it was changed elsewhere, so it's back)`);
    } else if (ti === undefined) {
      out = mi;
      conflicts.push(`${label(mi)} (deleted elsewhere, but you'd changed it, so it's kept)`);
    } else out = both(bi, mi, ti);
    if (out !== undefined) result.set(k, out);
  }
  const reordered = !sameOrder(base.map(key), mine.map(key));
  const [first, second] = reordered ? [mine, theirs] : [theirs, mine];
  const order = weave(first.map(key), second.map(key));
  return order.flatMap((k) => (result.has(k) ? [result.get(k)!] : []));
};

/** `primary`'s order, with what only `secondary` has placed after its neighbour there (or first). */
const weave = (primary: string[], secondary: string[]) => {
  const out = [...primary];
  secondary.forEach((k, i) => {
    if (out.includes(k)) return;
    const before = secondary.slice(0, i).findLast((p) => out.includes(p));
    out.splice(before === undefined ? 0 : out.indexOf(before) + 1, 0, k);
  });
  return out;
};

/** Whether the keys both lists share are in the same order. */
const sameOrder = (a: string[], b: string[]) => {
  const shared = new Set(a.filter((k) => b.includes(k)));
  const [x, y] = [a.filter((k) => shared.has(k)), b.filter((k) => shared.has(k))];
  return x.every((k, i) => k === y[i]);
};

const mergeMap = (
  base: Record<string, string>,
  mine: Record<string, string>,
  theirs: Record<string, string>,
  label: (key: string) => string,
  conflicts: string[],
): Record<string, string> => {
  const out: Record<string, string> = {};
  for (const k of new Set([...Object.keys(mine), ...Object.keys(theirs), ...Object.keys(base)])) {
    const [b, m, t] = [base[k], mine[k], theirs[k]];
    let v: string | undefined;
    if (m === b) v = t;
    else if (t === b || m === t) v = m;
    else {
      // Both changed it: this tab's value, or its edit if one side deleted it.
      v = m ?? t;
      conflicts.push(label(k));
    }
    if (v !== undefined) out[k] = v;
  }
  return out;
};

/** Set-like lists (a collection's groups): additions and removals from both sides. */
const mergeSet = (base: string[], mine: string[], theirs: string[]) => {
  const removed = new Set(base.filter((g) => !mine.includes(g) || !theirs.includes(g)));
  return [...new Set([...theirs, ...mine])].filter((g) => !removed.has(g));
};

/** The remaining fields of an object, one by one; `skip` are merged by the caller. */
const mergeFields = <T extends object>(
  base: T,
  mine: T,
  theirs: T,
  skip: (keyof T)[],
  label: string,
  conflicts: string[],
): T => {
  const out = { ...theirs };
  for (const k of new Set([...Object.keys(mine), ...Object.keys(theirs)]) as Set<keyof T>) {
    if (skip.includes(k)) continue;
    const [b, m, t] = [base[k], mine[k], theirs[k]];
    if (same(m, b)) out[k] = t;
    else if (same(t, b) || same(m, t)) out[k] = m;
    else {
      out[k] = m;
      conflicts.push(`${String(k)} of ${label}`);
    }
  }
  return out;
};

/** Deep equality for JSON values, ignoring key order (this tab and the engine order keys differently). */
export const same = (a: unknown, b: unknown): boolean => {
  if (a === b) return true;
  if (a === null || b === null || typeof a !== "object" || typeof b !== "object") {
    // `undefined` and a missing key mean the same thing as `null` in what the engine sends back.
    return (a ?? null) === (b ?? null);
  }
  if (Array.isArray(a) !== Array.isArray(b)) return false;
  if (Array.isArray(a)) {
    const bb = b as unknown[];
    return a.length === bb.length && a.every((x, i) => same(x, bb[i]));
  }
  const [ao, bo] = [a as Record<string, unknown>, b as Record<string, unknown>];
  const keys = new Set([...Object.keys(ao), ...Object.keys(bo)]);
  return [...keys].every((k) => same(ao[k], bo[k]));
};
