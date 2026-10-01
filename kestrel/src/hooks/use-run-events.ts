"use client";

import { useEffect } from "react";

import { errorMessage, getReport, runEventsUrl } from "@/lib/client";
import type { RunEvent } from "@/types/engine/RunEvent";
import { useRunStore } from "@/stores/run-store";

/**
 * Streams the attached run's events into the run store. The engine replays history on connect,
 * and `EventSource` resends `Last-Event-ID` when it reconnects, so the chart survives both page
 * reloads and dropped connections. On `finished`, the report is fetched over REST.
 */
export function useRunEvents() {
  const runId = useRunStore((s) => s.runId);

  useEffect(() => {
    // A finished run shown from history has nothing left to stream.
    if (!runId || useRunStore.getState().status !== "running") return;
    const { applyEvent, setReport, setError } = useRunStore.getState();
    // With credentials: the session cookie, when accounts are on and the UI is on another origin.
    const source = new EventSource(runEventsUrl(runId), { withCredentials: true });

    source.onmessage = (message) => {
      const event = JSON.parse(message.data) as RunEvent;
      applyEvent(event);
      if (event.type === "finished") {
        source.close();
        getReport(runId)
          .then(setReport)
          .catch((err) => setError(errorMessage(err)));
      }
    };

    source.onerror = () => {
      // CONNECTING means the browser is retrying on its own; CLOSED means it gave up.
      if (source.readyState === EventSource.CLOSED) {
        setError("Lost the event stream from the engine.");
      }
    };

    return () => source.close();
  }, [runId]);
}
