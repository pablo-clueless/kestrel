"use client";

import { ShieldOff } from "lucide-react";
import Link from "next/link";

import { WORKSPACE_PATH } from "@/hooks/use-to-workspace";
import { useMe } from "@/hooks/use-me";
import { Button } from "../ui/button";

/** Shows the admin pages to this engine's admins (`KESTREL_ADMIN_EMAILS`) only. The engine refuses
 * everyone else's admin calls anyway; this just explains instead of showing errors. Goes inside
 * `AuthGate`, so who's signed in is already known. */
export const AdminGate = ({ children }: { children: React.ReactNode }) => {
  const me = useMe().data;
  if (me?.user?.admin) return children;

  const reason =
    me?.auth === "off"
      ? "Accounts are off on this engine (KESTREL_AUTH=off), so there are no users to manage."
      : "Only this engine's admins can open these pages. Whoever runs it adds admins with KESTREL_ADMIN_EMAILS.";
  return (
    <div className="flex h-dvh flex-col items-center justify-center gap-3 p-4 text-center">
      <ShieldOff className="text-muted-foreground size-6" />
      <h1 className="text-lg font-semibold">Admin pages</h1>
      <p className="text-muted-foreground max-w-sm text-sm">{reason}</p>
      <Button variant="outline" render={<Link href={WORKSPACE_PATH} />} nativeButton={false}>
        Back to workspace
      </Button>
    </div>
  );
};
