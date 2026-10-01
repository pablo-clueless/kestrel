"use client";

import { useMe } from "@/hooks/use-me";
import { Input } from "../ui/input";
import { Label } from "../ui/label";
import { TabPanel } from "../shared";

interface Props {
  selected: string;
}

export const Account = ({ selected }: Props) => {
  const user = useMe().data?.user;

  return (
    <TabPanel selected={selected} value="account">
      <div className="bg-background flex flex-col gap-4 p-5">
        {user ? (
          <div className="flex max-w-sm flex-col gap-1.5">
            <Label htmlFor="account-email">Email</Label>
            <Input id="account-email" value={user.email} readOnly />
            <p className="text-muted-foreground text-xs">
              You sign in with this address. Changing it isn&apos;t supported yet.
            </p>
          </div>
        ) : (
          <p className="text-muted-foreground text-sm">
            Accounts are off on this engine: this browser&apos;s workspace is kept for it alone,
            without signing in.
          </p>
        )}
      </div>
    </TabPanel>
  );
};
