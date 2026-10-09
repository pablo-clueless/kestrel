"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { toast } from "sonner";
import {
  BadgeCheck,
  Ban,
  CircleCheck,
  KeyRound,
  LogOut,
  MoreHorizontal,
  Search,
  Trash2,
} from "lucide-react";

import { AdminPage, Badge, QueryState } from "@/components/admin";
import { ConfirmButton } from "@/components/shared";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import type { AdminUser } from "@/types/engine/AdminUser";
import { date, int, relative } from "@/lib/format";
import {
  adminDeleteUser,
  adminResetTwoFactor,
  adminSetDisabled,
  adminSignOutUser,
  adminUsers,
  adminVerifyEmail,
  errorMessage,
} from "@/lib/client";

const USERS_KEY = ["admin", "users"] as const;

const matches = (u: AdminUser, q: string) =>
  u.email.includes(q) || (u.name?.toLowerCase().includes(q) ?? false) || u.id.startsWith(q);

const Page = () => {
  const users = useQuery({ queryKey: USERS_KEY, queryFn: adminUsers });
  const [query, setQuery] = useState("");
  const q = query.trim().toLowerCase();
  const shown = useMemo(
    () => (users.data ?? []).filter((u) => !q || matches(u, q)),
    [users.data, q],
  );

  return (
    <AdminPage
      title="Users"
      description="Every account on this engine, newest first."
      actions={
        <div className="relative w-64">
          <Search className="text-muted-foreground pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2" />
          <Input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search by email, name or id"
            aria-label="Search users"
            className="pl-8"
          />
        </div>
      }
    >
      <QueryState query={users}>
        {() => (
          <div className="bg-card overflow-x-auto rounded-xs border">
            <table className="w-full text-sm">
              <thead className="text-muted-foreground border-b text-left text-xs">
                <tr>
                  <th className="px-4 py-2.5 font-medium">User</th>
                  <th className="px-4 py-2.5 font-medium">Security</th>
                  <th className="px-4 py-2.5 text-right font-medium">Workspaces</th>
                  <th className="px-4 py-2.5 font-medium">Last active</th>
                  <th className="px-4 py-2.5 font-medium">Joined</th>
                  <th className="w-12 px-4 py-2.5" aria-label="Actions" />
                </tr>
              </thead>
              <tbody className="divide-y">
                {shown.map((u) => (
                  <UserRow key={u.id} user={u} />
                ))}
              </tbody>
            </table>
            {shown.length === 0 && (
              <p className="text-muted-foreground px-4 py-8 text-center text-sm">
                {q ? `No users match “${query.trim()}”.` : "No accounts yet."}
              </p>
            )}
            <p className="text-muted-foreground border-t px-4 py-2 text-xs">
              {q
                ? `${int(shown.length)} of ${int(users.data!.length)} users`
                : `${int(users.data!.length)} users`}
            </p>
          </div>
        )}
      </QueryState>
    </AdminPage>
  );
};

const UserRow = ({ user: u }: { user: AdminUser }) => (
  <tr className={u.disabled ? "bg-muted/40" : undefined}>
    <td className="px-4 py-2.5">
      <div className="flex min-w-0 flex-col">
        <span className="flex flex-wrap items-center gap-1.5 font-medium">
          {u.name ?? u.email}
          {u.you && <Badge tone="primary">You</Badge>}
          {u.admin && <Badge>Admin</Badge>}
          {u.disabled && <Badge tone="destructive">Disabled</Badge>}
        </span>
        {u.name && <span className="text-muted-foreground text-xs">{u.email}</span>}
      </div>
    </td>
    <td className="px-4 py-2.5">
      <div className="flex flex-wrap gap-1.5">
        {u.emailVerified ? <Badge>Verified</Badge> : <Badge tone="warning">Unverified</Badge>}
        {u.twoFactor && <Badge>2FA</Badge>}
      </div>
    </td>
    <td className="px-4 py-2.5 text-right tabular-nums">{int(u.workspaces)}</td>
    <td className="text-muted-foreground px-4 py-2.5 text-xs whitespace-nowrap">
      {u.lastSeenAtMs === null ? "–" : relative(u.lastSeenAtMs)}
      {u.liveSessions > 0 && (
        <span className="block">
          {u.liveSessions} {u.liveSessions === 1 ? "session" : "sessions"}
        </span>
      )}
    </td>
    <td className="text-muted-foreground px-4 py-2.5 text-xs whitespace-nowrap">
      {date(u.createdAtMs)}
    </td>
    <td className="px-4 py-2.5 text-right">
      <UserActions user={u} />
    </td>
  </tr>
);

const item = "w-full justify-start rounded-none px-2.5 font-normal";

const UserActions = ({ user: u }: { user: AdminUser }) => {
  const queryClient = useQueryClient();
  const [open, setOpen] = useState(false);
  const act = useMutation({
    mutationFn: async (action: () => Promise<string>) => action(),
    onSuccess: (message) => {
      toast.success(message);
      setOpen(false);
      void queryClient.invalidateQueries({ queryKey: ["admin"] });
    },
    onError: (err) => toast.error(errorMessage(err)),
  });
  // Lockout actions aren't offered for yourself or another admin: the engine refuses them, since
  // KESTREL_ADMIN_EMAILS is where admins are managed.
  const protectedUser = u.you || u.admin;
  const who = u.name ?? u.email;

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger
        render={<Button variant="ghost" size="icon-sm" aria-label={`Actions for ${u.email}`} />}
      >
        <MoreHorizontal className="size-4" />
      </PopoverTrigger>
      <PopoverContent align="end" className="w-60 gap-0 p-0 py-1">
        {!u.emailVerified && (
          <Button
            variant="ghost"
            className={item}
            disabled={act.isPending}
            onClick={() =>
              act.mutate(async () => {
                await adminVerifyEmail(u.id);
                return `Marked ${u.email} as verified.`;
              })
            }
          >
            <BadgeCheck className="size-3.5" /> Mark email verified
          </Button>
        )}
        {u.twoFactor && (
          <ConfirmButton
            variant="ghost"
            className={item}
            pending={act.isPending}
            confirmLabel="Click again to turn it off"
            onConfirm={() =>
              act.mutate(async () => {
                await adminResetTwoFactor(u.id);
                return `Two-factor sign-in is off for ${who}.`;
              })
            }
          >
            <KeyRound className="size-3.5" /> Turn off two-factor
          </ConfirmButton>
        )}
        {protectedUser ? (
          <p className="text-muted-foreground px-2.5 py-1.5 text-xs">
            {u.you
              ? "Manage your own sign-in from your profile."
              : "An admin of this engine. Remove them from KESTREL_ADMIN_EMAILS to manage their account here."}
          </p>
        ) : (
          <>
            {u.liveSessions > 0 && (
              <ConfirmButton
                variant="ghost"
                className={item}
                pending={act.isPending}
                confirmLabel="Click again to sign out"
                onConfirm={() =>
                  act.mutate(async () => {
                    const ended = await adminSignOutUser(u.id);
                    return `Signed ${who} out of ${ended} ${ended === 1 ? "session" : "sessions"}.`;
                  })
                }
              >
                <LogOut className="size-3.5" /> Sign out everywhere
              </ConfirmButton>
            )}
            {u.disabled ? (
              <Button
                variant="ghost"
                className={item}
                disabled={act.isPending}
                onClick={() =>
                  act.mutate(async () => {
                    await adminSetDisabled(u.id, false);
                    return `${who} can sign in again.`;
                  })
                }
              >
                <CircleCheck className="size-3.5" /> Enable account
              </Button>
            ) : (
              <ConfirmButton
                variant="ghost"
                className={item}
                pending={act.isPending}
                confirmLabel="Click again to disable"
                onConfirm={() =>
                  act.mutate(async () => {
                    await adminSetDisabled(u.id, true);
                    return `Disabled ${who}; they've been signed out.`;
                  })
                }
              >
                <Ban className="size-3.5" /> Disable account
              </ConfirmButton>
            )}
            <ConfirmButton
              variant="ghost"
              className={`${item} text-destructive hover:text-destructive border-t`}
              pending={act.isPending}
              confirmLabel="Click again to delete"
              onConfirm={() =>
                act.mutate(async () => {
                  await adminDeleteUser(u.id);
                  return `Deleted ${who}.`;
                })
              }
            >
              <Trash2 className="size-3.5" /> Delete account
            </ConfirmButton>
            <p className="text-muted-foreground px-2.5 pt-1 pb-1.5 text-xs">
              Deleting also deletes the workspaces only they are in.
            </p>
          </>
        )}
      </PopoverContent>
    </Popover>
  );
};

export default Page;
