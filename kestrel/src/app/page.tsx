import { EnvironmentEditor, RequestEditor, ResponseView } from "@/components/workspace";
import { Card, Header, Sidebar } from "@/components/shared";
import {
  DistributionChart,
  LatencyChart,
  LatencyStats,
  ReportSummary,
  RunControls,
  ThroughputChart,
} from "@/components/run";

const Page = () => {
  return (
    <div className="flex h-screen w-screen overflow-hidden">
      <Sidebar />
      <div className="flex min-w-0 flex-1 flex-col">
        <Header />
        <main className="h-[calc(100vh-64px)] flex-1 overflow-y-auto p-4">
          <div className="grid grid-cols-4 gap-4">
            <Card title="Request" className="col-span-2">
              <RequestEditor />
            </Card>
            <Card title="Environment">
              <EnvironmentEditor />
            </Card>
            <Card title="Latency">
              <LatencyStats />
            </Card>
            <Card title="Run">
              <RunControls />
            </Card>
            <Card title="Response">
              <ResponseView />
            </Card>
            <Card title="Report">
              <ReportSummary />
            </Card>
            <Card title="Request/Time" className="col-span-2"></Card>
            <Card title="Live latency" className="col-span-2">
              <LatencyChart />
            </Card>
            <Card title="Throughput" className="col-span-2">
              <ThroughputChart />
            </Card>

            <Card title="Distribution" className="col-span-2">
              <DistributionChart />
            </Card>
          </div>
        </main>
      </div>
    </div>
  );
};

export default Page;
