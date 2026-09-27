import { create } from "zustand";

import { errorMessage, getWorkspace, putWorkspace, setSecret } from "@/lib/client";
import type { Endpoint } from "@/types/engine/Endpoint";
import type { Workspace } from "@/types/engine/Workspace";

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

export const useWorkspaceStore = create<WorkspaceState>()((set, get) => {
  /** Applies an edit and schedules a save. */
  const edit = (fn: (ws: Workspace) => Workspace) => {
    const ws = get().workspace;
    if (!ws) return;
    set({ workspace: fn(ws), saveState: "saving" });
    clearTimeout(saveTimer);
    // Failures land in `saveError`; nothing to do with the rejection here.
    saveTimer = setTimeout(() => get().flush().catch(() => {}), SAVE_DEBOUNCE_MS);
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
          selectedId: workspace.endpoints[0]?.id ?? null,
          loadError: null,
        });
      } catch (err) {
        set({ loadError: errorMessage(err) });
      }
    },

    select: (selectedId) => set({ selectedId }),

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
      };
      edit((ws) => ({ ...ws, endpoints: [...ws.endpoints, endpoint] }));
      set({ selectedId: endpoint.id });
    },

    updateEndpoint: (id, patch) =>
      edit((ws) => ({
        ...ws,
        endpoints: ws.endpoints.map((e) => (e.id === id ? { ...e, ...patch } : e)),
      })),

    removeEndpoint: (id) => {
      edit((ws) => ({ ...ws, endpoints: ws.endpoints.filter((e) => e.id !== id) }));
      if (get().selectedId === id) set({ selectedId: get().workspace?.endpoints[0]?.id ?? null });
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
  useWorkspaceStore((s) => s.workspace?.endpoints.find((e) => e.id === s.selectedId) ?? null);
