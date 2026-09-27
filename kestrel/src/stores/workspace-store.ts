import { create } from "zustand";

import { errorMessage, getWorkspace, putWorkspace, setSecret } from "@/lib/client";
import type { Collection } from "@/types/engine/Collection";
import type { Workspace } from "@/types/engine/Workspace";
import type { Endpoint } from "@/types/engine/Endpoint";

const SAVE_DEBOUNCE_MS = 400;

type SaveState = "idle" | "saving" | "saved" | "error";

interface WorkspaceState {
  workspace: Workspace | null;
  /** Environment → names of secrets set. Values never leave the engine. */
  secretKeys: Record<string, string[]>;
  selectedId: string | null;
  saveState: SaveState;
  saveError: string | null;
  loadError: string | null;

  load: () => Promise<void>;
  select: (id: string | null) => void;
  /** Switches the sidebar to another collection and selects its first endpoint. */
  setActiveCollection: (id: string) => void;
  addCollection: (name: string) => void;
  /** Adds a collection made by the engine (spec import), makes it active and selects its first endpoint. */
  addImportedCollection: (collection: Collection) => void;
  renameCollection: (id: string, name: string) => void;
  removeCollection: (id: string) => void;
  setCollectionVar: (id: string, key: string, value: string | null) => void;
  /** Adds to the active collection (creating a "Default" one if there are none). */
  addEndpoint: () => void;
  updateEndpoint: (id: string, patch: Partial<Endpoint>) => void;
  removeEndpoint: (id: string) => void;
  setActiveEnvironment: (name: string | null) => void;
  addEnvironment: (name: string) => void;
  removeEnvironment: (name: string) => void;
  setVar: (env: string, key: string, value: string | null) => void;
  setSecret: (env: string, key: string, value: string | null) => Promise<void>;
  /** Writes pending edits now. Runs call this so the engine sees what's on screen. */
  flush: () => Promise<void>;
}

let saveTimer: ReturnType<typeof setTimeout> | undefined;

/** The collection shown in the sidebar: the saved one, else the first. */
export const activeCollectionOf = (ws: Workspace | null): Collection | null =>
  ws?.collections.find((c) => c.id === ws.activeCollection) ?? ws?.collections[0] ?? null;

const newCollection = (name: string): Collection => ({
  id: crypto.randomUUID(),
  name,
  vars: {},
  endpoints: [],
  source: null,
  schemaDefs: null,
});

/** Applies `fn` to every collection's endpoint list. */
const mapEndpoints = (ws: Workspace, fn: (endpoints: Endpoint[]) => Endpoint[]): Workspace => ({
  ...ws,
  collections: ws.collections.map((c) => ({ ...c, endpoints: fn(c.endpoints) })),
});

export const useWorkspaceStore = create<WorkspaceState>()((set, get) => {
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
    saveState: "idle",
    saveError: null,
    loadError: null,

    load: async () => {
      try {
        const { workspace, secretKeys } = await getWorkspace();
        set({
          workspace,
          secretKeys,
          selectedId: activeCollectionOf(workspace)?.endpoints[0]?.id ?? null,
          loadError: null,
        });
      } catch (err) {
        set({ loadError: errorMessage(err) });
      }
    },

    select: (selectedId) => set({ selectedId }),

    setActiveCollection: (id) => {
      edit((ws) => ({ ...ws, activeCollection: id }));
      const collection = get().workspace?.collections.find((c) => c.id === id);
      if (!collection?.endpoints.some((e) => e.id === get().selectedId)) {
        set({ selectedId: collection?.endpoints[0]?.id ?? null });
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
      set({ selectedId: collection.endpoints[0]?.id ?? null });
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
      const active = activeCollectionOf(get().workspace);
      if (!active?.endpoints.some((e) => e.id === get().selectedId)) {
        set({ selectedId: active?.endpoints[0]?.id ?? null });
      }
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

    addEndpoint: () => {
      const endpoint: Endpoint = {
        id: crypto.randomUUID(),
        name: "New endpoint",
        group: null,
        method: "GET",
        url: "{{base}}/",
        headers: [],
        query: [],
        body: { type: "none" },
        auth: { type: "none" },
        expect: null,
      };
      edit((ws) => {
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
      });
      set({ selectedId: endpoint.id });
    },

    updateEndpoint: (id, patch) =>
      edit((ws) =>
        mapEndpoints(ws, (eps) => eps.map((e) => (e.id === id ? { ...e, ...patch } : e))),
      ),

    removeEndpoint: (id) => {
      edit((ws) => mapEndpoints(ws, (eps) => eps.filter((e) => e.id !== id)));
      if (get().selectedId === id) {
        set({ selectedId: activeCollectionOf(get().workspace)?.endpoints[0]?.id ?? null });
      }
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

export const useSelectedEndpoint = () =>
  useWorkspaceStore(
    (s) =>
      s.workspace?.collections.flatMap((c) => c.endpoints).find((e) => e.id === s.selectedId) ??
      null,
  );

export const useActiveCollection = () => useWorkspaceStore((s) => activeCollectionOf(s.workspace));
