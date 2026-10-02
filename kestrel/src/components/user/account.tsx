"use client";

import { BadgeCheck, CircleAlert } from "lucide-react";
import { useMutation } from "@tanstack/react-query";
import { toast } from "sonner";

import { errorMessage, resendVerification } from "@/lib/client";
import { useMe } from "@/hooks/use-me";
import { Button } from "../ui/button";
import { TabPanel } from "../shared";
import { Input } from "../ui/input";
import { Label } from "../ui/label";

interface Props {
  selected: string;
}

export const Account = ({ selected }: Props) => {
  const me = useMe().data;
  const user = me?.user;
  const resend = useMutation({
    mutationFn: resendVerification,
    onSuccess: () => toast.success(`Sent a new link to ${user?.email}. It works for 24 hours.`),
    onError: (err) => toast.error(errorMessage(err)),
  });

  return (
    <TabPanel selected={selected} value="account">
      <div className="bg-background flex flex-col gap-4 p-5">
        {user ? (
          <div className="flex max-w-sm flex-col gap-1.5">
            <Label htmlFor="account-email">Email</Label>
            <Input id="account-email" value={user.email} readOnly />
            {/* Without email on this engine there's no way to verify, so the status means nothing. */}
            {me.mail &&
              (user.emailVerified ? (
                <p className="text-success flex items-center gap-1.5 text-xs">
                  <BadgeCheck className="size-3.5" /> Verified
                </p>
              ) : (
                <div className="flex flex-col items-start gap-2">
                  <p className="text-warning flex items-center gap-1.5 text-xs">
                    <CircleAlert className="size-3.5" /> Not verified yet. Open the link we emailed
                    you when you signed up.
                  </p>
                  <Button
                    variant="outline"
                    size="sm"
                    disabled={resend.isPending}
                    onClick={() => resend.mutate()}
                  >
                    {resend.isPending ? "Sending…" : "Send link again"}
                  </Button>
                </div>
              ))}
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
