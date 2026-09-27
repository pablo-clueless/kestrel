import { EndpointList } from "../workspace";
import { cn } from "cn";

/**
 * user can paste or import endpoints from the API spec here.  (M3: OpenAPI import)
 * user can paste or import multiple endpoints at once.        (M3)
 * user can import endpoints from the API spec file.           (M3)
 * user can import endpoints from the API spec file in JSON/YAML format. (M3)
 * endpoints are stored in the store.                          (done: workspace store → kestrel.json)
 * endpoints are displayed in the sidebar.                     (done)
 * user can click on the endpoint to view the details.         (done: Request card)
 * user can click on the "Run" button to run the endpoint.     (done: Run card)
 */
export const Sidebar = () => {
  return (
    <aside className={cn("h-full w-68 shrink-0 border-r p-4")}>
      <EndpointList />
    </aside>
  );
};
