import { create } from "zustand";

import type { RunConfig } from "@/types/engine/RunConfig";
import type { RunReport } from "@/types/engine/RunReport";
import type { RunStatus } from "@/types/engine/RunStatus";
import type { RunEvent } from "@/types/engine/RunEvent";
import type { Bucket } from "@/types/engine/Bucket";
import type { ComplexityProgress } from "@/types/engine/ComplexityProgress";

/** 600 buckets × 250 ms = the last 2.5 minutes on screen. */
const MAX_BUCKETS = 600;

interface RunState {
  runId: string | null;
  status: RunStatus | null;
  config: RunConfig | null;
  buckets: Bucket[];
  /** Big-O runs: medians per size so far. */
  complexity: ComplexityProgress | null;
  report: RunReport | null;
  error: string | null;

  /** Point the store at a run. Clears everything; the SSE history replay refills it. */
  attach: (runId: string) => void;
  applyEvent: (event: RunEvent) => void;
  setReport: (report: RunReport) => void;
  setError: (error: string | null) => void;
  /** Drop the shown run (e.g. the endpoint or test type changed). No-op while a run is live. */
  clear: () => void;
}

export const useRunStore = create<RunState>()((set) => ({
  runId: null,
  status: null,
  config: null,
  buckets: [],
  complexity: null,
  report: null,
  error: null,

  attach: (runId) =>
    set({
      runId,
      status: "running",
      config: null,
      buckets: [],
      complexity: null,
      report: null,
      error: null,
    }),

  applyEvent: (event) =>
    set((state) => {
      switch (event.type) {
        case "started":
          return { config: event.config, status: "running" };
        case "bucket": {
          const buckets = [...state.buckets, event];
          return { buckets: buckets.length > MAX_BUCKETS ? buckets.slice(-MAX_BUCKETS) : buckets };
        }
        case "complexity":
          return { complexity: event };
        case "resync":
          // The engine's full retained history follows immediately.
          return { buckets: [], complexity: null };
        case "finished":
          return { status: event.status };
      }
    }),

  setReport: (report) => set({ report }),
  setError: (error) => set({ error }),
  clear: () =>
    set((state) =>
      state.status === "running"
        ? state
        : {
            runId: null,
            status: null,
            config: null,
            buckets: [],
            complexity: null,
            report: null,
            error: null,
          },
    ),
}));
