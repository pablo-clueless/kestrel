"use client";

import { Bell, Fingerprint, Palette, User, type LucideIcon } from "lucide-react";
import { useState } from "react";

import { Account, Appearance, Notifications, Security } from "@/components/user";
import type { ProfileTab } from "@/stores/layout-store";
import { cn } from "cn";

const TABS: { id: ProfileTab; label: string; icon: LucideIcon }[] = [
  { id: "account", label: "Account", icon: User },
  { id: "security", label: "Security", icon: Fingerprint },
  { id: "appearance", label: "Appearance", icon: Palette },
  { id: "notifications", label: "Notifications", icon: Bell },
];

const Page = () => {
  const [tab, setTab] = useState<ProfileTab>("account");

  return (
    <div className="h-full overflow-y-auto">
      <div className="flex h-full w-full flex-col gap-6 p-5">
        <div>
          <h1 className="text-lg font-semibold">Profile</h1>
          <p className="text-muted-foreground text-sm">Your account, sign-in and preferences.</p>
        </div>
        <div className="bg-card h-[calc(100%-48px)] overflow-hidden rounded-xs border">
          <div role="tablist" aria-label="Profile sections" className="flex">
            {TABS.map(({ id, label, icon: Icon }) => (
              <button
                key={id}
                role="tab"
                id={`tab-${id}`}
                aria-selected={tab === id}
                aria-controls={`tabpanel-${id}`}
                tabIndex={tab === id ? 0 : -1}
                onClick={() => setTab(id)}
                className={cn(
                  "flex flex-1 items-center justify-center gap-1.5 px-2 py-2.5 text-xs transition-colors duration-150",
                  tab === id
                    ? "bg-background text-foreground font-medium"
                    : "text-muted-foreground hover:text-foreground hover:bg-muted/60 border-b",
                )}
              >
                <Icon className="size-3.5" />
                {label}
              </button>
            ))}
          </div>
          <Account selected={tab} />
          <Security selected={tab} />
          <Appearance selected={tab} />
          <Notifications selected={tab} />
        </div>
      </div>
    </div>
  );
};

export default Page;
