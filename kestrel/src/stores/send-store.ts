import { create } from "zustand";

import { errorMessage, sendRequest } from "@/lib/client";
import type { Endpoint } from "@/types/engine/Endpoint";
import type { SendResponse } from "@/types/engine/SendResponse";
import { useWorkspaceStore } from "./workspace-store";

interface SendState {
  pending: boolean;
  result: SendResponse | null;
  error: string | null;
  send: (endpoint: Endpoint, environment: string | null) => Promise<void>;
  /** Drop the shown response, and ignore any send still in flight. */
  clear: () => void;
}

/** Bumped by each send and clear, so a stale response can't land after the selection changed. */
let generation = 0;

/** The last "Send" (try it) result, shown in the Response card. */
export const useSendStore = create<SendState>()((set) => ({
  pending: false,
  result: null,
  error: null,
  send: async (endpoint, environment) => {
    const current = ++generation;
    set({ pending: true, error: null });
    try {
      const result = await sendRequest({ endpoint, environment, timeoutMs: null });
      // The engine already stored any secrets, so reflect the saves even if the result is stale.
      if (environment) useWorkspaceStore.getState().applySaved(environment, result.saved);
      if (current === generation) set({ result, pending: false });
    } catch (err) {
      if (current === generation) set({ error: errorMessage(err), result: null, pending: false });
    }
  },
  clear: () => {
    generation++;
    set({ pending: false, result: null, error: null });
  },
}));
