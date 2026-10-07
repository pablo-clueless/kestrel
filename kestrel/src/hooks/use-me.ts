import { useQuery } from "@tanstack/react-query";

import { getMe } from "@/lib/client";

export const ME_KEY = ["me"] as const;

/** Who's signed in (and whether accounts are on at all). Fetched once per page load: sign-in and
 * sign-out update or reload it themselves, and an ended session surfaces as a 401 elsewhere. */
export const useMe = () =>
  useQuery({ queryKey: ME_KEY, queryFn: getMe, staleTime: Infinity, retry: 1 });

/** The workspace the app is acting on, with the user's role in it. `undefined` with accounts off. */
export const useCurrentWorkspace = () => {
  const me = useMe().data;
  return me?.workspaces.find((w) => w.id === me.workspaceId);
};

/** Whether the user may change the current workspace and send requests from it: everyone but a
 * member with read access (and everyone, with accounts off). The engine enforces it; this only shapes the UI. */
export const useCanEdit = () => useCurrentWorkspace()?.role !== "read";
