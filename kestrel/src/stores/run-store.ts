import { create } from "zustand";

import type { Bucket } from "@/types/engine/Bucket";
import type { RunConfig } from "@/types/engine/RunConfig";
import type { RunEvent } from "@/types/engine/RunEvent";
import type { RunReport } from "@/types/engine/RunReport";
import type { RunStatus } from "@/types/engine/RunStatus";

/** 600 buckets × 250 ms = the last 2.5 minutes on screen. */
const MAX_BUCKETS = 600;

interface RunState {
  runId: string | null;
  status: RunStatus | null;
  config: RunConfig | null;
  buckets: Bucket[];
  report: RunReport | null;
  error: string | null;

  /** Point the store at a run. Clears everything; the SSE history replay refills it. */
  attach: (runId: string) => void;
  applyEvent: (event: RunEvent) => void;
  setReport: (report: RunReport) => void;
  setError: (error: string | null) => void;
}

export const useRunStore = create<RunState>()((set) => ({
  runId: null,
  status: null,
  config: null,
  buckets: [],
  report: null,
  error: null,

  attach: (runId) =>
    set({ runId, status: "running", config: null, buckets: [], report: null, error: null }),

  applyEvent: (event) =>
    set((state) => {
      switch (event.type) {
        case "started":
          return { config: event.config, status: "running" };
        case "bucket": {
          const buckets = [...state.buckets, event];
          return { buckets: buckets.length > MAX_BUCKETS ? buckets.slice(-MAX_BUCKETS) : buckets };
        }
        case "resync":
          // The engine's full retained history follows immediately.
          return { buckets: [] };
        case "finished":
          return { status: event.status };
      }
    }),

  setReport: (report) => set({ report }),
  setError: (error) => set({ error }),
}));
