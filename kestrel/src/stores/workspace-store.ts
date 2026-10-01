import { create } from "zustand";

import { errorMessage, getWorkspace, putWorkspace, setSecret } from "@/lib/client";
import type { Collection } from "@/types/engine/Collection";
import type { Workspace } from "@/types/engine/Workspace";
import type { Endpoint } from "@/types/engine/Endpoint";
import type { Saved } from "@/types/engine/Saved";

const SAVE_DEBOUNCE_MS = 400;

type SaveState = "idle" | "saving" | "saved" | "error";

interface WorkspaceState {
  workspace: Workspace | null;
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
}

let saveTimer: ReturnType<typeof setTimeout> | undefined;

/** The collection shown in the sidebar: the saved one, else the first. */
export const activeCollectionOf = (ws: Workspace | null): Collection | null =>
  ws?.collections.find((c) => c.id === ws.activeCollection) ?? ws?.collections[0] ?? null;

/** `openIds` with `id` appended if it isn't already open. */
const withTab = (openIds: string[], id: string | null) =>
  id === null || openIds.includes(id) ? openIds : [...openIds, id];

const newEndpoint = (name: string): Endpoint => ({
  id: crypto.randomUUID(),
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
  id: crypto.randomUUID(),
  name,
  vars: {},
  endpoints: [],
  groups: [],
  source: null,
  schemaDefs: null,
});

/** A collection's groups in sidebar order: those made by hand (empty ones included), then any
 * others its endpoints use, e.g. imported tags. */
export const groupsOf = (collection: Collection): string[] => [
  ...new Set([
    ...collection.groups,
    ...collection.endpoints.flatMap((e) => (e.group ? [e.group] : [])),
  ]),
];

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
    if (selectedId !== null && !ids.has(selectedId)) {
      select(openIds.at(-1) ?? activeCollectionOf(get().workspace)?.endpoints[0]?.id ?? null);
    }
  };

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
    secretKeys: {},
    selectedId: null,
    openIds: [],
    drafts: {},
    saveState: "idle",
    saveError: null,
    loadError: null,

    load: async () => {
      try {
        const { workspace, secretKeys } = await getWorkspace();
        const first = activeCollectionOf(workspace)?.endpoints[0]?.id ?? null;
        set({
          workspace,
          secretKeys,
          selectedId: first,
          openIds: first ? [first] : [],
          loadError: null,
        });
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

    setActiveCollection: (id) => {
      edit((ws) => ({ ...ws, activeCollection: id }));
      const collection = get().workspace?.collections.find((c) => c.id === id);
      const { selectedId, drafts } = get();
      if (selectedId !== null && selectedId in drafts) return;
      if (!collection?.endpoints.some((e) => e.id === get().selectedId)) {
        select(collection?.endpoints[0]?.id ?? null);
      }
    },

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

    setActiveEnvironment: (name) => edit((ws) => ({ ...ws, activeEnvironment: name })),

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

    flush: async () => {
      clearTimeout(saveTimer);
      const ws = get().workspace;
      if (!ws || get().saveState !== "saving") return;
      try {
        await putWorkspace(ws);
        // Only mark saved if nothing changed while the request was in flight.
        if (get().workspace === ws) set({ saveState: "saved", saveError: null });
      } catch (err) {
        set({ saveState: "error", saveError: errorMessage(err) });
        throw err;
      }
    },
  };
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
