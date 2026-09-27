import { create } from "zustand";

import { errorMessage, sendRequest } from "@/lib/client";
import type { Endpoint } from "@/types/engine/Endpoint";
import type { Sample } from "@/types/engine/Sample";

interface SendState {
  pending: boolean;
  result: Sample | null;
  error: string | null;
  send: (endpoint: Endpoint, environment: string | null) => Promise<void>;
}

/** The last "Send" (try it) result, shown in the Response card. */
export const useSendStore = create<SendState>()((set) => ({
  pending: false,
  result: null,
  error: null,
  send: async (endpoint, environment) => {
    set({ pending: true, error: null });
    try {
      const result = await sendRequest({ endpoint, environment, timeoutMs: null });
      set({ result, pending: false });
    } catch (err) {
      set({ error: errorMessage(err), result: null, pending: false });
    }
  },
}));
