"use client";

import { WorkspaceSettings } from "@/components/team/workspace-settings";
import { useMe } from "@/hooks/use-me";

const Page = () => {
  const accountsOn = useMe().data?.auth === "on";

  return (
    <div className="h-full overflow-y-auto">
      <div className="flex w-full flex-col gap-6 p-5">
        <div>
          <h1 className="text-lg font-semibold">Settings</h1>
          <p className="text-muted-foreground text-sm">
            The workspace you&apos;re in, and who&apos;s in it.
          </p>
        </div>
        <div className="bg-card rounded-xs border p-5">
          {accountsOn ? (
            <WorkspaceSettings />
          ) : (
            <p className="text-muted-foreground text-sm">
              Accounts are off on this engine, so this browser&apos;s workspace is yours alone and
              there&apos;s no one to share it with. Turn on accounts (<code>KESTREL_AUTH=on</code>)
              to invite people.
            </p>
          )}
        </div>
      </div>
    </div>
  );
};

export default Page;
