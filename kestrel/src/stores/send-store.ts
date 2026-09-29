import { create } from "zustand";

import { errorMessage, sendRequest } from "@/lib/client";
import type { Endpoint } from "@/types/engine/Endpoint";
import type { SendResponse } from "@/types/engine/SendResponse";
import { useWorkspaceStore } from "./workspace-store";

export interface SendEntry {
  pending: boolean;
  result: SendResponse | null;
  error: string | null;
}

const EMPTY: SendEntry = { pending: false, result: null, error: null };

interface SendState {
  /** Endpoint id → its last Send, so each tab keeps its own response. */
  byId: Record<string, SendEntry>;
  send: (endpoint: Endpoint, environment: string | null) => Promise<void>;
  /** Drop an endpoint's response, and ignore any send of it still in flight. */
  clear: (id: string) => void;
}

/** Endpoint id → counter bumped by each send and clear, so a stale response can't land. */
const generations = new Map<string, number>();
const bump = (id: string) => {
  const next = (generations.get(id) ?? 0) + 1;
  generations.set(id, next);
  return next;
};

/** The last "Send" (try it) result per endpoint, shown in the Response card. */
export const useSendStore = create<SendState>()((set) => {
  const put = (id: string, entry: SendEntry) => set((s) => ({ byId: { ...s.byId, [id]: entry } }));

  return {
    byId: {},
    send: async (endpoint, environment) => {
      const { id } = endpoint;
      const current = bump(id);
      put(id, { pending: true, result: null, error: null });
      try {
        const result = await sendRequest({ endpoint, environment, timeoutMs: null });
        // The engine already stored any secrets, so reflect the saves even if the result is stale.
        if (environment) useWorkspaceStore.getState().applySaved(environment, result.saved);
        if (current === generations.get(id)) put(id, { result, pending: false, error: null });
      } catch (err) {
        if (current === generations.get(id)) {
          put(id, { error: errorMessage(err), result: null, pending: false });
        }
      }
    },
    clear: (id) => {
      bump(id);
      set((s) => {
        const byId = { ...s.byId };
        delete byId[id];
        return { byId };
      });
    },
  };
});

// A closed tab's response goes with it.
useWorkspaceStore.subscribe((ws, prev) => {
  if (ws.openIds === prev.openIds) return;
  const { byId, clear } = useSendStore.getState();
  for (const id of Object.keys(byId)) if (!ws.openIds.includes(id)) clear(id);
});

/** The Send state of one endpoint (idle if it was never sent). */
export const useSendEntry = (id: string | null | undefined) =>
  useSendStore((s) => (id ? (s.byId[id] ?? EMPTY) : EMPTY));
