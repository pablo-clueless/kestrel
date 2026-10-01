import React from "react";

interface Props {
  children: React.ReactNode;
}

export default function AuthLayout({ children }: Props) {
  return (
    <div className="bg-background grid min-h-dvh w-full place-items-center px-4 py-10">
      <div className="bg-card w-full max-w-sm rounded-xs border p-6">{children}</div>
    </div>
  );
}
