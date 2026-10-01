import { RequestEditor, RequestTabs, ResponseView } from "@/components/workspace";
import { LiveSummary, ResultsSummary, RunControls } from "@/components/run";
import { Card, StatusBar } from "@/components/shared";

/**
 * Three columns: the main content takes two and scrolls; the Run panel takes one and scrolls on its
 * own, so its height changing (test type, run details) never shifts the main column.
 */
const Page = () => {
  return (
    <div className="flex h-full flex-col">
      {/* `grid-rows-1` is minmax(0, 1fr): the row takes the space left above the status bar instead
          of growing to its content, so both columns scroll inside it. */}
      <div className="grid min-h-0 flex-1 grid-cols-4 grid-rows-1">
        <main className="col-span-3 flex min-h-0 min-w-0 flex-col">
          <RequestTabs />
          <div className="flex min-h-0 flex-1 flex-col gap-5 overflow-y-auto p-5">
            <Card title="Request">
              <RequestEditor />
              <section className="mt-5 flex flex-col gap-3 border-t pt-5">
                <h3 className="text-sm font-semibold">Response</h3>
                <ResponseView />
              </section>
            </Card>
            <Card title="Live" info="Updated every 250 ms while a run is in progress.">
              <LiveSummary />
            </Card>
            <Card title="Results" info="From the final report, fetched when the run finishes.">
              <ResultsSummary />
            </Card>
          </div>
        </main>
        <aside className="bg-card col-span-1 min-h-0 border-l">
          <RunControls />
        </aside>
      </div>
      <StatusBar />
    </div>
  );
};

export default Page;
