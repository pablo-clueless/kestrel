import { create } from "zustand";
import { toast } from "sonner";

import {
  errorMessage,
  getWorkspace,
  putWorkspace,
  setSecret,
  workspaceConflict,
  workspaceId,
} from "@/lib/client";
import { mergeWorkspace, same } from "@/lib/merge";
import type { Collection } from "@/types/engine/Collection";
import type { KeyValue } from "@/types/engine/KeyValue";
import type { Workspace } from "@/types/engine/Workspace";
import type { Endpoint } from "@/types/engine/Endpoint";
import type { Saved } from "@/types/engine/Saved";
import { generateUUID } from "@/lib/utils";

const SAVE_DEBOUNCE_MS = 400;
/** Saves refused because someone else saved first are merged and retried this many times. */
const SAVE_ATTEMPTS = 4;

type SaveState = "idle" | "saving" | "saved" | "error";

interface WorkspaceState {
  workspace: Workspace | null;
  /** The workspace as the engine last had it, as far as this tab knows, and its revision: what
   * edits are made on. A save sends the revision, so the engine refuses it if someone else saved
   * meanwhile, and the edits are merged into theirs (see `mergeWorkspace`). */
  base: Workspace | null;
  revision: number;
  /** Environment → names of secrets set. Values never leave the engine. */
  secretKeys: Record<string, string[]>;
  selectedId: string | null;
  /** Endpoints open as tabs, in tab order. The selected endpoint is always among them. */
  openIds: string[];
  /** Unsaved requests opened with a new tab. Not part of the workspace until saved. */
  drafts: Record<string, Endpoint>;
  saveState: SaveState;
  saveError: string | null;
  loadError: string | null;

  load: () => Promise<void>;
  /** Shows an endpoint, opening a tab for it if it has none. */
  select: (id: string | null) => void;
  /** Closes a tab; closing the selected one selects its neighbour, as a browser does. */
  closeTab: (id: string) => void;
  /** Sets the tab order (drag to reorder). */
  reorderTabs: (ids: string[]) => void;
  /** Opens a tab with a new draft request, without adding it to any collection. */
  newTab: () => void;
  /** Moves a draft into the active collection; its tab stays open. */
  saveDraft: (id: string) => void;
  /** Switches the sidebar to another collection and selects its first endpoint. */
  setActiveCollection: (id: string) => void;
  addCollection: (name: string) => void;
  /** Adds a collection made by the engine (spec import), makes it active and selects its first endpoint. */
  addImportedCollection: (collection: Collection) => void;
  renameCollection: (id: string, name: string) => void;
  removeCollection: (id: string) => void;
  setCollectionVar: (id: string, key: string, value: string | null) => void;
  /** Headers sent with every endpoint in the collection; an endpoint's own header wins. */
  setCollectionHeaders: (id: string, headers: KeyValue[]) => void;
  /** Adds a group to a collection; it stays listed while empty. No-op if the name is taken. */
  addGroup: (collectionId: string, name: string) => void;
  /** Removes a group; its endpoints stay in the collection, ungrouped. */
  removeGroup: (collectionId: string, name: string) => void;
  /** Adds to the active collection (creating a "Default" one if there are none), in `group` if given. */
  addEndpoint: (group?: string) => void;
  updateEndpoint: (id: string, patch: Partial<Endpoint>) => void;
  removeEndpoint: (id: string) => void;
  setActiveEnvironment: (name: string | null) => void;
  addEnvironment: (name: string) => void;
  removeEnvironment: (name: string) => void;
  setVar: (env: string, key: string, value: string | null) => void;
  setSecret: (env: string, key: string, value: string | null) => Promise<void>;
  /** Reflects what a Send's extract rules saved: variables are set here, secrets were stored by the engine. */
  applySaved: (env: string, saved: Saved[]) => void;
  /** Writes pending edits now. Runs call this so the engine sees what's on screen. */
  flush: () => Promise<void>;
  /** Picks up what others saved (shared workspaces). Skipped while this tab has edits to save:
   * saving merges theirs in anyway. */
  refresh: () => Promise<void>;
}

let saveTimer: ReturnType<typeof setTimeout> | undefined;
/** Saves run one at a time: two in flight would carry the same base revision, and the second would
 * be refused by the first. */
let saving: Promise<void> = Promise.resolve();

/** The active collection and environment are each person's own: kept in this browser per workspace,
 * never in the shared document. Otherwise a teammate picking their prod environment would pick it
 * for you too, and the engine (which falls back to the saved one) would send your requests there. */
const viewKey = () => `kestrel-view:${workspaceId()}`;

type View = { environment: string | null; collection: string | null };

const readView = (): View | null => {
  try {
    const raw = localStorage.getItem(viewKey());
    return raw ? (JSON.parse(raw) as View) : null;
  } catch {
    return null;
  }
};

const writeView = (ws: Workspace) => {
  try {
    const view: View = { environment: ws.activeEnvironment, collection: ws.activeCollection };
    localStorage.setItem(viewKey(), JSON.stringify(view));
  } catch {
    // Storage blocked: the view lasts for this tab only.
  }
};

/** The workspace as it's saved: everything but this person's view. */
const withoutView = (ws: Workspace): Workspace => ({
  ...ws,
  activeEnvironment: null,
  activeCollection: null,
});

/** `request "Login", variable "token" in environment "local"`, for a toast. */
const listOf = (items: string[]) =>
  items.length <= 3
    ? items.join(", ")
    : `${items.slice(0, 3).join(", ")} and ${items.length - 3} more`;

/** The collection shown in the sidebar: the saved one, else the first. */
export const activeCollectionOf = (ws: Workspace | null): Collection | null =>
  ws?.collections.find((c) => c.id === ws.activeCollection) ?? ws?.collections[0] ?? null;

/** `openIds` with `id` appended if it isn't already open. */
const withTab = (openIds: string[], id: string | null) =>
  id === null || openIds.includes(id) ? openIds : [...openIds, id];

const newEndpoint = (name: string): Endpoint => ({
  id: generateUUID(),
  name,
  group: null,
  method: "GET",
  url: "",
  headers: [],
  query: [],
  body: { type: "none" },
  auth: { type: "none" },
  expect: null,
  extract: [],
});

const newCollection = (name: string): Collection => ({
  id: generateUUID(),
  name,
  vars: {},
  headers: [],
  endpoints: [],
  groups: [],
  source: null,
  schemaDefs: null,
});

/** Secret values shorter than this aren't scrubbed from bodies; matches the engine's
 * `redact::MIN_SECRET_LEN`. */
const MIN_SCRUBBED_SECRET = 3;

/** Name order for the sidebar: ignores case, and puts numbers in order (`v2` before `v10`). */
export const byName = new Intl.Collator(undefined, { sensitivity: "base", numeric: true });

/** A collection's groups, alphabetically (case-insensitive, `v2` before `v10`): those made by hand
 * (empty ones included) and any others its endpoints use, e.g. imported tags. */
export const groupsOf = (collection: Collection): string[] =>
  [
    ...new Set([
      ...collection.groups,
      ...collection.endpoints.flatMap((e) => (e.group ? [e.group] : [])),
    ]),
  ].sort(byName.compare);

/** Applies `fn` to one collection. */
const mapCollection = (
  ws: Workspace,
  id: string,
  fn: (c: Collection) => Collection,
): Workspace => ({
  ...ws,
  collections: ws.collections.map((c) => (c.id === id ? fn(c) : c)),
});

/** Appends to the active collection (creating a "Default" one if there are none) and makes it active. */
const insertEndpoint = (ws: Workspace, endpoint: Endpoint): Workspace => {
  const target = activeCollectionOf(ws) ?? newCollection("Default");
  const exists = ws.collections.some((c) => c.id === target.id);
  const withEndpoint = { ...target, endpoints: [...target.endpoints, endpoint] };
  return {
    ...ws,
    collections: exists
      ? ws.collections.map((c) => (c.id === target.id ? withEndpoint : c))
      : [...ws.collections, withEndpoint],
    activeCollection: target.id,
  };
};

/** Applies `fn` to every collection's endpoint list. */
const mapEndpoints = (ws: Workspace, fn: (endpoints: Endpoint[]) => Endpoint[]): Workspace => ({
  ...ws,
  collections: ws.collections.map((c) => ({ ...c, endpoints: fn(c.endpoints) })),
});

export const useWorkspaceStore = create<WorkspaceState>()((set, get) => {
  const select = (id: string | null) =>
    set((s) => ({ selectedId: id, openIds: withTab(s.openIds, id) }));

  /** Drops tabs whose endpoint no longer exists, then keeps the selection valid. */
  const pruneTabs = () => {
    const ids = new Set([
      ...(get().workspace?.collections.flatMap((c) => c.endpoints.map((e) => e.id)) ?? []),
      ...Object.keys(get().drafts),
    ]);
    const openIds = get().openIds.filter((id) => ids.has(id));
    set({ openIds });
    const { selectedId } = get();
    // Fall back to another open tab, never to an endpoint nobody opened.
    if (selectedId !== null && !ids.has(selectedId)) select(openIds.at(-1) ?? null);
  };

  /** Changes only this person's view (active collection or environment): nothing to save. */
  const setView = (patch: Partial<Pick<Workspace, "activeCollection" | "activeEnvironment">>) =>
    set((s) => (s.workspace ? { workspace: { ...s.workspace, ...patch } } : s));

  /** Applies an edit and schedules a save. */
  const edit = (fn: (ws: Workspace) => Workspace) => {
    const ws = get().workspace;
    if (!ws) return;
    set({ workspace: fn(ws), saveState: "saving" });
    clearTimeout(saveTimer);
    // Failures land in `saveError`; nothing to do with the rejection here.
    saveTimer = setTimeout(
      () =>
        get()
          .flush()
          .catch(() => {}),
      SAVE_DEBOUNCE_MS,
    );
  };

  return {
    workspace: null,
    base: null,
    revision: 0,
    secretKeys: {},
    selectedId: null,
    openIds: [],
    drafts: {},
    saveState: "idle",
    saveError: null,
    loadError: null,

    load: async () => {
      try {
        const { workspace, secretKeys, revision } = await getWorkspace();
        // This browser's view of the workspace, or (the first time) whatever the document used to
        // carry. Names that no longer exist are dropped: an unknown environment fails every send.
        const view = readView() ?? {
          environment: workspace.activeEnvironment,
          collection: workspace.activeCollection,
        };
        const local: Workspace = {
          ...workspace,
          activeEnvironment: workspace.environments.some((e) => e.name === view.environment)
            ? view.environment
            : null,
          activeCollection: workspace.collections.some((c) => c.id === view.collection)
            ? view.collection
            : null,
        };
        // Nothing is opened for you: the dashboard starts blank until you pick an endpoint.
        set({
          workspace: local,
          base: workspace,
          revision,
          secretKeys,
          selectedId: null,
          openIds: [],
          loadError: null,
        });
        // A document saved before the view moved out of it still names an environment, which the
        // engine falls back to when a request names none. Save it without, once.
        if (workspace.activeEnvironment !== null || workspace.activeCollection !== null) {
          edit((ws) => ws);
        }
      } catch (err) {
        set({ loadError: errorMessage(err) });
      }
    },

    select,

    closeTab: (id) => {
      const { openIds, selectedId } = get();
      const index = openIds.indexOf(id);
      if (index === -1) return;
      const rest = openIds.filter((t) => t !== id);
      const drafts = { ...get().drafts };
      delete drafts[id];
      set({
        openIds: rest,
        drafts,
        selectedId: selectedId === id ? (rest[index] ?? rest[index - 1] ?? null) : selectedId,
      });
    },

    reorderTabs: (openIds) => set({ openIds }),

    newTab: () => {
      const draft = newEndpoint("Untitled request");
      set((s) => ({ drafts: { ...s.drafts, [draft.id]: draft } }));
      select(draft.id);
    },

    saveDraft: (id) => {
      const draft = get().drafts[id];
      if (!draft) return;
      edit((ws) => insertEndpoint(ws, draft));
      set((s) => {
        const drafts = { ...s.drafts };
        delete drafts[id];
        return { drafts };
      });
    },

    // Only the active collection changes; the selection is left to whoever called (selecting or
    // adding an endpoint), so opening a collection never picks an endpoint for you.
    setActiveCollection: (id) => setView({ activeCollection: id }),

    addCollection: (name) => {
      const collection = newCollection(name);
      edit((ws) => ({
        ...ws,
        collections: [...ws.collections, collection],
        activeCollection: collection.id,
      }));
      set({ selectedId: null });
    },

    addImportedCollection: (collection) => {
      edit((ws) => ({
        ...ws,
        collections: [...ws.collections, collection],
        activeCollection: collection.id,
      }));
      select(collection.endpoints[0]?.id ?? null);
    },

    renameCollection: (id, name) =>
      edit((ws) => ({
        ...ws,
        collections: ws.collections.map((c) => (c.id === id ? { ...c, name } : c)),
      })),

    removeCollection: (id) => {
      edit((ws) => {
        const collections = ws.collections.filter((c) => c.id !== id);
        const activeCollection =
          ws.activeCollection === id ? (collections[0]?.id ?? null) : ws.activeCollection;
        return { ...ws, collections, activeCollection };
      });
      pruneTabs();
    },

    setCollectionVar: (id, key, value) =>
      edit((ws) => ({
        ...ws,
        collections: ws.collections.map((c) => {
          if (c.id !== id) return c;
          const vars = { ...c.vars };
          if (value === null) delete vars[key];
          else vars[key] = value;
          return { ...c, vars };
        }),
      })),

    setCollectionHeaders: (id, headers) =>
      edit((ws) => ({
        ...ws,
        collections: ws.collections.map((c) => (c.id === id ? { ...c, headers } : c)),
      })),

    addGroup: (collectionId, name) =>
      edit((ws) =>
        mapCollection(ws, collectionId, (c) => {
          const taken = groupsOf(c).some((g) => g.toLowerCase() === name.toLowerCase());
          return taken ? c : { ...c, groups: [...c.groups, name] };
        }),
      ),

    removeGroup: (collectionId, name) =>
      edit((ws) =>
        mapCollection(ws, collectionId, (c) => ({
          ...c,
          groups: c.groups.filter((g) => g !== name),
          endpoints: c.endpoints.map((e) => (e.group === name ? { ...e, group: null } : e)),
        })),
      ),

    addEndpoint: (group) => {
      const endpoint = { ...newEndpoint("New endpoint"), group: group ?? null };
      edit((ws) => insertEndpoint(ws, endpoint));
      select(endpoint.id);
    },

    updateEndpoint: (id, patch) => {
      const draft = get().drafts[id];
      if (draft) {
        set((s) => ({ drafts: { ...s.drafts, [id]: { ...draft, ...patch } } }));
        return;
      }
      edit((ws) =>
        mapEndpoints(ws, (eps) => eps.map((e) => (e.id === id ? { ...e, ...patch } : e))),
      );
    },

    removeEndpoint: (id) => {
      edit((ws) => mapEndpoints(ws, (eps) => eps.filter((e) => e.id !== id)));
      get().closeTab(id);
      pruneTabs();
    },

    setActiveEnvironment: (name) => setView({ activeEnvironment: name }),

    addEnvironment: (name) =>
      edit((ws) =>
        ws.environments.some((e) => e.name === name)
          ? ws
          : {
              ...ws,
              environments: [...ws.environments, { name, vars: {} }],
              activeEnvironment: ws.activeEnvironment ?? name,
            },
      ),

    removeEnvironment: (name) =>
      edit((ws) => ({
        ...ws,
        environments: ws.environments.filter((e) => e.name !== name),
        activeEnvironment: ws.activeEnvironment === name ? null : ws.activeEnvironment,
      })),

    setVar: (env, key, value) =>
      edit((ws) => ({
        ...ws,
        environments: ws.environments.map((e) => {
          if (e.name !== env) return e;
          const vars = { ...e.vars };
          if (value === null) delete vars[key];
          else vars[key] = value;
          return { ...e, vars };
        }),
      })),

    setSecret: async (env, key, value) => {
      try {
        await setSecret({ environment: env, key, value });
        if (value !== null && value.length > 0 && value.length < MIN_SCRUBBED_SECRET) {
          toast.warning(
            `"${key}" is shorter than ${MIN_SCRUBBED_SECRET} characters, so it isn't hidden in response bodies: it would match ordinary text. It's still hidden in Authorization, Cookie and API-key headers.`,
          );
        }
        set((s) => {
          const keys = new Set(s.secretKeys[env] ?? []);
          if (value === null) keys.delete(key);
          else keys.add(key);
          return { secretKeys: { ...s.secretKeys, [env]: [...keys].sort() } };
        });
      } catch (err) {
        set({ saveState: "error", saveError: errorMessage(err) });
      }
    },

    applySaved: (env, saved) => {
      const ok = saved.filter((s) => s.error === null);
      for (const s of ok) {
        if (s.target === "variable" && s.value !== null) get().setVar(env, s.name, s.value);
      }
      const secrets = ok.filter((s) => s.target === "secret").map((s) => s.name);
      if (secrets.length === 0) return;
      set((s) => {
        const keys = new Set([...(s.secretKeys[env] ?? []), ...secrets]);
        return { secretKeys: { ...s.secretKeys, [env]: [...keys].sort() } };
      });
    },

    flush: () => {
      clearTimeout(saveTimer);
      const run = saving.then(save, save);
      saving = run.catch(() => {});
      return run;
    },

    refresh: async () => {
      const { saveState, revision } = get();
      if (saveState === "saving" || !get().workspace) return;
      const current = await getWorkspace();
      // Re-checked after the round trip: an edit made meanwhile will merge theirs in when it saves.
      if (current.revision === revision || get().saveState === "saving") return;
      // Nothing unsaved here (`saving` would be set), so theirs is the new state, except for this
      // tab's view: someone else switching to their prod environment mustn't switch this tab's
      // sends there. (Merging with nothing changed on this side does exactly that.)
      const mine = get().workspace ?? current.workspace;
      set({
        workspace: mergeWorkspace(mine, mine, current.workspace).merged,
        base: current.workspace,
        revision: current.revision,
        secretKeys: current.secretKeys,
      });
      pruneTabs();
    },
  };

  /** Writes what's on screen, merging in anyone else's save that got there first. */
  async function save() {
    if (get().saveState !== "saving") return;
    const conflicts = new Set<string>();
    for (let attempt = 1; ; attempt++) {
      const ws = get().workspace;
      if (!ws) return;
      try {
        const revision = await putWorkspace(withoutView(ws), get().revision);
        set({ base: withoutView(ws), revision });
        // Only mark saved if nothing changed while the request was in flight.
        if (get().workspace === ws) set({ saveState: "saved", saveError: null });
        break;
      } catch (err) {
        const conflict = workspaceConflict(err);
        if (!conflict || attempt === SAVE_ATTEMPTS) {
          set({ saveState: "error", saveError: errorMessage(err) });
          throw err;
        }
        const theirs = conflict.current;
        // Edits made while the save was in flight are in `workspace` too, so they're merged as well.
        const { merged, conflicts: clashes } = mergeWorkspace(
          get().base ?? theirs.workspace,
          get().workspace ?? ws,
          theirs.workspace,
        );
        clashes.forEach((c) => conflicts.add(c));
        set({
          workspace: merged,
          base: theirs.workspace,
          revision: theirs.revision,
          secretKeys: theirs.secretKeys,
        });
        pruneTabs();
        // Theirs already has everything: nothing of ours to write.
        if (same(withoutView(merged), theirs.workspace)) {
          set({ saveState: "saved", saveError: null });
          break;
        }
      }
    }
    if (conflicts.size > 0) {
      toast.warning(
        `Someone else changed the same things at the same time. Your version was kept for ${listOf([...conflicts])}.`,
      );
    }
  }
});

/** The selected endpoint, saved or draft. */
export const useSelectedEndpoint = () =>
  useWorkspaceStore((s) =>
    s.selectedId === null
      ? null
      : (s.drafts[s.selectedId] ??
        s.workspace?.collections.flatMap((c) => c.endpoints).find((e) => e.id === s.selectedId) ??
        null),
  );

/** Whether the selected endpoint is an unsaved draft. */
export const useSelectedIsDraft = () =>
  useWorkspaceStore((s) => s.selectedId !== null && s.selectedId in s.drafts);

export const useActiveCollection = () => useWorkspaceStore((s) => activeCollectionOf(s.workspace));

// Remembers the view whenever it changes, whichever action changed it (picking an environment,
// adding a collection, a merge dropping one that was deleted elsewhere).
useWorkspaceStore.subscribe((s, prev) => {
  const [now, before] = [s.workspace, prev.workspace];
  if (
    now &&
    (now.activeEnvironment !== before?.activeEnvironment ||
      now.activeCollection !== before?.activeCollection)
  ) {
    writeView(now);
  }
});
