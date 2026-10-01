import React from "react";

import { AuthGate, Header, Sidebar } from "@/components/shared";

interface Props {
  children: React.ReactNode;
}

export default function DashboardLayout({ children }: Props) {
  return (
    <AuthGate>
      {/* Fills the viewport and never scrolls itself: each page fills the space under the header
          and scrolls its own regions, so the header (and the workspace's status bar) stay put. */}
      <div className="flex h-dvh w-full overflow-hidden">
        <Sidebar />
        <div className="flex min-w-0 flex-1 flex-col">
          <Header />
          <div className="min-h-0 flex-1">{children}</div>
        </div>
      </div>
    </AuthGate>
  );
}
