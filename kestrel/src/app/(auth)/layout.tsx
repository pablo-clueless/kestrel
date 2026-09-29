import React from "react";

interface Props {
  children: React.ReactNode;
}

export default function AuthLayout({ children }: Props) {
  return (
    <div className="bg-background grid h-screen w-screen place-items-center">
      <div className="bg-card min-w-125 border p-4">{children}</div>
    </div>
  );
}
