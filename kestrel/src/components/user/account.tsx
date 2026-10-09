"use client";

import { BadgeCheck, CircleAlert, Copy, Trash2 } from "lucide-react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { toast } from "sonner";

import {
  deleteAccount,
  errorMessage,
  resendVerification,
  setProfile,
  twoFactorStatus,
} from "@/lib/client";
import { ME_KEY, useMe, useCurrentWorkspace } from "@/hooks/use-me";
import type { UserInfo } from "@/types/engine/UserInfo";
import { ConfirmButton, TabPanel } from "../shared";
import { Button } from "../ui/button";
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
      <div className="bg-background flex h-[calc(100%-36px)] flex-col gap-4 overflow-y-auto p-5">
        {user ? (
          <>
            {/* Keyed by the saved name, so the field resets to it once a save lands. */}
            <DisplayName key={user.name ?? ""} user={user} />
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
                      <CircleAlert className="size-3.5" /> Not verified yet. Open the link we
                      emailed you when you signed up.
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
            <div className="flex max-w-sm flex-col gap-1.5">
              <Label htmlFor="account-id">User ID</Label>
              <div className="flex gap-2">
                <Input id="account-id" value={user.id} readOnly className="font-mono text-xs" />
                <Button
                  variant="outline"
                  size="icon"
                  className="shrink-0"
                  onClick={() => {
                    void navigator.clipboard.writeText(user.id);
                    toast.success("Copied");
                  }}
                >
                  <Copy className="size-3.5" />
                </Button>
              </div>
            </div>
            <WorkspacesSection />
            <DeleteAccount />
          </>
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

const ROLE_LABEL: Record<string, string> = { admin: "Admin", write: "Editor", read: "Viewer" };

const WorkspacesSection = () => {
  const me = useMe().data;
  const current = useCurrentWorkspace();
  if (!me?.workspaces.length) return null;
  return (
    <section className="flex flex-col gap-2">
      <h2 className="text-sm font-semibold">Workspaces</h2>
      <ul className="divide-y rounded-xs border">
        {me.workspaces.map((w) => (
          <li key={w.id} className="flex items-center justify-between gap-3 px-3 py-2.5 text-sm">
            <div className="flex min-w-0 flex-col">
              <span className="flex items-center gap-2 font-medium">
                {w.name}
                {w.id === current?.id && (
                  <span className="bg-primary/15 text-primary rounded-full px-2 py-0.5 text-[11px] font-medium">
                    Current
                  </span>
                )}
              </span>
              <span className="text-muted-foreground text-xs">
                {w.members} {w.members === 1 ? "member" : "members"}
              </span>
            </div>
            <span className="text-muted-foreground shrink-0 text-xs">
              {ROLE_LABEL[w.role] ?? w.role}
            </span>
          </li>
        ))}
      </ul>
    </section>
  );
};

const NAME_MAX = 80;

const DisplayName = ({ user }: { user: UserInfo }) => {
  const queryClient = useQueryClient();
  const [name, setName] = useState(user.name ?? "");
  const save = useMutation({
    mutationFn: () => setProfile({ name }),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ME_KEY });
      toast.success(name.trim() ? "Name saved." : "Name removed.");
    },
    onError: (err) => toast.error(errorMessage(err)),
  });
  const changed = name.trim() !== (user.name ?? "");
  const tooLong = name.trim().length > NAME_MAX;

  return (
    <form
      className="flex max-w-sm flex-col gap-1.5"
      onSubmit={(e) => {
        e.preventDefault();
        if (changed && !tooLong) save.mutate();
      }}
    >
      <Label htmlFor="account-name">Name</Label>
      <div className="flex gap-2">
        <Input
          id="account-name"
          value={name}
          placeholder={user.email.split("@")[0]}
          autoComplete="name"
          aria-invalid={tooLong}
          onChange={(e) => setName(e.target.value)}
        />
        <Button type="submit" variant="outline" disabled={!changed || tooLong || save.isPending}>
          {save.isPending ? "Saving…" : "Save"}
        </Button>
      </div>
      <p className={tooLong ? "text-destructive text-xs" : "text-muted-foreground text-xs"}>
        {tooLong
          ? `Keep it to ${NAME_MAX} characters.`
          : "Shown to people in your workspaces and in invites you send. Leave it empty to show your email."}
      </p>
    </form>
  );
};

const DeleteAccount = () => {
  const [open, setOpen] = useState(false);
  const [password, setPassword] = useState("");
  const [code, setCode] = useState("");
  // Only asked once the form is open; with two-factor on, deleting needs a code too.
  const twoFactor = useQuery({ queryKey: ["two-factor"], queryFn: twoFactorStatus, enabled: open });
  const needsCode = twoFactor.data?.enabled ?? false;
  const remove = useMutation({
    mutationFn: () => deleteAccount({ password, code: needsCode ? code : undefined }),
    // A full load: the session is gone, and nothing of this account should stay in memory.
    // eslint-disable-next-line @next/next/no-location-assign-relative-destination
    onSuccess: () => window.location.assign("/"),
    onError: (err) => toast.error(errorMessage(err)),
  });

  return (
    <section className="flex max-w-sm flex-col gap-3 border-t pt-4">
      <div>
        <h2 className="text-destructive text-sm font-semibold">Delete account</h2>
        <p className="text-muted-foreground text-sm">
          Deletes your account and every workspace only you are in, with their collections, secrets
          and run history. Workspaces you share stay with the people in them. This can&apos;t be
          undone.
        </p>
      </div>
      {open ? (
        <div className="flex flex-col gap-3">
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="delete-password">Password</Label>
            <Input
              id="delete-password"
              type="password"
              autoComplete="current-password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
            />
          </div>
          {needsCode && (
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="delete-code">Authenticator or recovery code</Label>
              <Input
                id="delete-code"
                autoComplete="one-time-code"
                value={code}
                onChange={(e) => setCode(e.target.value)}
              />
            </div>
          )}
          <div className="flex gap-2">
            <ConfirmButton
              variant="destructive"
              disabled={!password || (needsCode && !code.trim()) || twoFactor.isPending}
              pending={remove.isPending}
              pendingLabel="Deleting…"
              confirmLabel="Click again to delete"
              onConfirm={() => remove.mutate()}
            >
              <Trash2 className="size-3.5" /> Delete my account
            </ConfirmButton>
            <Button
              variant="outline"
              onClick={() => {
                setOpen(false);
                setPassword("");
                setCode("");
              }}
            >
              Cancel
            </Button>
          </div>
        </div>
      ) : (
        <Button variant="outline" className="self-start" onClick={() => setOpen(true)}>
          Delete account…
        </Button>
      )}
    </section>
  );
};
