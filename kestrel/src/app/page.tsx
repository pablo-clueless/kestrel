import { LiveSummary, ResultsSummary, RunControls } from "@/components/run";
import { Card, Header, Sidebar, StatusBar } from "@/components/shared";
import { RequestEditor, ResponseView } from "@/components/workspace";

/**
 * Three columns: the main content takes two and scrolls; the Run panel takes one and scrolls on its
 * own, so its height changing (test type, run details) never shifts the main column.
 */
const Page = () => {
  return (
    <div className="flex h-screen w-screen overflow-hidden">
      <Sidebar />
      <div className="flex min-w-0 flex-1 flex-col">
        <Header />
        <div className="grid min-h-0 flex-1 grid-cols-4">
          <main className="col-span-3 flex min-w-0 flex-col gap-5 overflow-y-auto p-5">
            <Card title="Request">
              <RequestEditor />
            </Card>
            <Card title="Response" info="The last Send. Credentials are redacted by the engine.">
              <ResponseView />
            </Card>
            <Card title="Live" info="Updated every 250 ms while a run is in progress.">
              <LiveSummary />
            </Card>
            <Card title="Results" info="From the final report, fetched when the run finishes.">
              <ResultsSummary />
            </Card>
          </main>
          <aside className="bg-card col-span-1 min-h-0 border-l">
            <RunControls />
          </aside>
        </div>
        <StatusBar />
      </div>
    </div>
  );
};

export default Page;
