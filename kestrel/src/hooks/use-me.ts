import { useQuery } from "@tanstack/react-query";

import { getMe } from "@/lib/client";

export const ME_KEY = ["me"] as const;

/** Who's signed in (and whether accounts are on at all). Fetched once per page load: sign-in and
 * sign-out update or reload it themselves, and an ended session surfaces as a 401 elsewhere. */
export const useMe = () =>
  useQuery({ queryKey: ME_KEY, queryFn: getMe, staleTime: Infinity, retry: 1 });
