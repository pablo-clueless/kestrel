import { usePathname, useRouter } from "next/navigation";
import { useCallback } from "react";

export const WORKSPACE_PATH = "/workspace";

/** Shows the workspace page, if another page (Settings, Profile) is open. The sidebar is on every
 * page, so opening a request or a run from it there should go where it's shown. */
export const useToWorkspace = () => {
  const pathname = usePathname();
  const router = useRouter();
  return useCallback(() => {
    if (pathname !== WORKSPACE_PATH) router.push(WORKSPACE_PATH);
  }, [pathname, router]);
};
