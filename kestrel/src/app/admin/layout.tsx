import React from "react";

import { AdminGate, Header, Sidebar } from "@/components/admin";
import { AuthGate } from "@/components/shared";

interface Props {
  children: React.ReactNode;
}

export default function AdminLayout({ children }: Props) {
  return (
    <AuthGate>
      <AdminGate>
        <div className="flex h-dvh w-full overflow-hidden">
          <Sidebar />
          <div className="flex min-w-0 flex-1 flex-col">
            <Header />
            <div className="h-[calc(100vh-56px)] min-h-0 flex-1">{children}</div>
          </div>
        </div>
      </AdminGate>
    </AuthGate>
  );
}
