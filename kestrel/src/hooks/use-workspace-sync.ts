import { useEffect } from "react";

import { useWorkspaceStore } from "@/stores/workspace-store";
import { workspaceEventsUrl } from "@/lib/client";

/** Keeps the workspace up to date with what others save (other members, or this user's other tabs):
 * the engine announces each new revision on an event stream, and the store fetches the workspace
 * when it's not the one it has. `EventSource` reconnects by itself after a dropped connection, and
 * the first event after (re)connecting is the current revision, so nothing saved meanwhile is missed. */
export const useWorkspaceSync = () => {
  const refresh = useWorkspaceStore((s) => s.refresh);

  useEffect(() => {
    const events = new EventSource(workspaceEventsUrl(), { withCredentials: true });
    events.addEventListener("revision", (e) => {
      const { revision } = JSON.parse((e as MessageEvent<string>).data) as { revision: number };
      // A revision this tab saved itself (or already has) needs nothing. A failed fetch is retried
      // with the next announcement; a removed member is sent home by the client's 404 handling.
      if (revision !== useWorkspaceStore.getState().revision) refresh().catch(() => {});
    });
    return () => events.close();
  }, [refresh]);
};
