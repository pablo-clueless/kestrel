"use client";

import { LogOut, Settings, ShieldCheck, User } from "lucide-react";
import Link from "next/link";
import { useState } from "react";
import { toast } from "sonner";

import { errorMessage, signOut } from "@/lib/client";
import { useMe } from "@/hooks/use-me";
import { Button } from "../ui/button";
import { cn } from "cn";
import {
  Popover,
  PopoverContent,
  PopoverDescription,
  PopoverHeader,
  PopoverTitle,
  PopoverTrigger,
} from "../ui/popover";

const item =
  "flex w-full items-center gap-2 px-2.5 py-1.5 text-left text-sm transition-colors hover:bg-muted";

/** The signed-in user's menu. Hidden when accounts are off: there's no one to sign out. */
export const LoggUser = () => {
  const [open, setOpen] = useState(false);
  const [signingOut, setSigningOut] = useState(false);
  const user = useMe().data?.user;
  if (!user) return null;
  const label = user.name ?? user.email;

  const onSignOut = async () => {
    setSigningOut(true);
    try {
      await signOut();
      // A full load, not router.push: the stores still hold this user's workspace and runs.
      // eslint-disable-next-line @next/next/no-location-assign-relative-destination
      window.location.assign("/");
    } catch (err) {
      toast.error(errorMessage(err));
      setSigningOut(false);
    }
  };

  return (
    <Popover onOpenChange={setOpen} open={open}>
      <PopoverTrigger
        render={
          <Button
            variant="outline"
            size="icon"
            aria-label={`Account: ${label}`}
            title={label}
            className="font-semibold uppercase"
          />
        }
      >
        {label.charAt(0)}
      </PopoverTrigger>
      <PopoverContent align="end" className="w-60 gap-0 p-0">
        <PopoverHeader className="border-b px-2.5 py-2">
          <PopoverTitle className="truncate text-sm" title={user.name ?? undefined}>
            {user.name ?? "Signed in as"}
          </PopoverTitle>
          <PopoverDescription className="truncate" title={user.email}>
            {user.email}
          </PopoverDescription>
        </PopoverHeader>
        <nav className="flex flex-col py-1">
          <Link href="/profile" className={item} onClick={() => setOpen(false)}>
            <User className="size-3.5" /> Profile
          </Link>
          <Link href="/settings" className={item} onClick={() => setOpen(false)}>
            <Settings className="size-3.5" /> Settings
          </Link>
          {user.admin && (
            <Link href="/admin/overview" className={item} onClick={() => setOpen(false)}>
              <ShieldCheck className="size-3.5" /> Admin
            </Link>
          )}
          <button
            type="button"
            className={cn(item, "border-t text-red-700 dark:text-red-400")}
            onClick={onSignOut}
            disabled={signingOut}
          >
            <LogOut className="size-3.5" /> {signingOut ? "Signing out…" : "Sign out"}
          </button>
        </nav>
      </PopoverContent>
    </Popover>
  );
};
