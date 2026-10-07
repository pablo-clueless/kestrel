"use client";

import { Check, Copy, LogOut, Mail, Plus, Trash2, UserMinus } from "lucide-react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { toast } from "sonner";
import { z } from "zod";

import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../ui/select";
import { RoleBadge } from "../shared/workspace-switcher";
import { ME_KEY, useCurrentWorkspace, useMe } from "@/hooks/use-me";
import type { InviteCreated } from "@/types/engine/InviteCreated";
import type { WorkspaceInfo } from "@/types/engine/WorkspaceInfo";
import type { MemberInfo } from "@/types/engine/MemberInfo";
import type { Role } from "@/types/engine/Role";
import { Button } from "../ui/button";
import { Form } from "../form";
import {
  createWorkspace,
  deleteWorkspace,
  errorMessage,
  inviteMember,
  listMembers,
  removeMember,
  renameWorkspace,
  revokeInvite,
  setMemberRole,
  switchWorkspace,
} from "@/lib/client";

const membersKey = (id: string) => ["members", id] as const;

const MEMBER_ROLES: { value: Role; label: string; hint: string }[] = [
  { value: "admin", label: "Admin", hint: "everything, including managing members" },
  {
    value: "write",
    label: "Write",
    hint: "changes requests, environments and secrets, and runs tests",
  },
  { value: "read", label: "Read", hint: "sees the workspace and results, changes nothing" },
];

/** Settings → the current workspace: its name, members and invites, the user's other workspaces, and
 * deleting it. Admins manage; everyone else sees who's there and can leave. */
export const WorkspaceSettings = () => {
  const current = useCurrentWorkspace();
  if (!current) return null;
  const admin = current.role === "admin";
  return (
    <div className="flex flex-col gap-8">
      <Name workspace={current} />
      <Members workspace={current} />
      {admin && <Invite workspace={current} />}
      <YourWorkspaces />
      {admin && <DeleteWorkspace workspace={current} />}
    </div>
  );
};

/** How an invite reads in the list: "Invited as an admin", "Invited with write access". */
const inviteAccess = (role: Role) =>
  role === "admin" ? "Invited as an admin" : `Invited with ${role} access`;

/** "alice@example.com", "alice@… and bob@…", "alice@… and 2 others". */
const adminList = (emails: string[]) =>
  emails.length === 0
    ? "nobody"
    : emails.length <= 2
      ? emails.join(" and ")
      : `${emails[0]} and ${emails.length - 1} others`;

const Section = ({
  title,
  description,
  children,
}: {
  title: string;
  description?: React.ReactNode;
  children: React.ReactNode;
}) => (
  <section className="flex flex-col gap-3">
    <div>
      <h2 className="text-sm font-semibold">{title}</h2>
      {description && <p className="text-muted-foreground text-sm">{description}</p>}
    </div>
    {children}
  </section>
);

const nameSchema = z.object({
  name: z.string().min(1, "Give the workspace a name").max(80, "Keep it to 80 characters"),
});

const Name = ({ workspace }: { workspace: WorkspaceInfo }) => {
  const queryClient = useQueryClient();
  const admin = workspace.role === "admin";
  return (
    <Section
      title="Workspace"
      description={
        admin
          ? "You're an admin here. Everyone in this workspace sees this name."
          : `Admins: ${adminList(workspace.adminEmails)}. You have ${workspace.role} access here.`
      }
    >
      <Form
        className="max-w-sm"
        schema={nameSchema}
        defaultValues={{ name: workspace.name }}
        toastOnInvalid={false}
        fields={{ name: { type: "text", label: "Name", readOnly: !admin } }}
        onSubmit={async ({ name }) => {
          try {
            await renameWorkspace(workspace.id, name);
            await queryClient.invalidateQueries({ queryKey: ME_KEY });
            toast.success("Renamed.");
          } catch (err) {
            toast.error(errorMessage(err));
          }
        }}
      >
        {({ field, isSubmitting }) => (
          <div className="flex flex-col gap-3">
            {field("name")}
            {admin && (
              <Button type="submit" className="self-start" disabled={isSubmitting}>
                {isSubmitting ? "Saving…" : "Save name"}
              </Button>
            )}
          </div>
        )}
      </Form>
    </Section>
  );
};

const Members = ({ workspace }: { workspace: WorkspaceInfo }) => {
  const queryClient = useQueryClient();
  const admin = workspace.role === "admin";
  const members = useQuery({
    queryKey: membersKey(workspace.id),
    queryFn: () => listMembers(workspace.id),
  });
  useEffect(() => {
    if (members.isError) toast.error(errorMessage(members.error));
  }, [members.isError, members.error]);
  const refresh = () => {
    void queryClient.invalidateQueries({ queryKey: membersKey(workspace.id) });
    void queryClient.invalidateQueries({ queryKey: ME_KEY });
  };
  const changeRole = useMutation({
    mutationFn: ({ userId, role }: { userId: string; role: Role }) =>
      setMemberRole(workspace.id, userId, role),
    onSuccess: refresh,
    onError: (err) => toast.error(errorMessage(err)),
  });
  const remove = useMutation({
    mutationFn: (member: MemberInfo) => removeMember(workspace.id, member.userId),
    onSuccess: (_, member) => {
      if (member.you) {
        // Left: the next page load lands in the default workspace.
        // eslint-disable-next-line @next/next/no-location-assign-relative-destination
        window.location.assign("/workspace");
        return;
      }
      toast.success(`Removed ${member.email}.`);
      refresh();
    },
    onError: (err) => toast.error(errorMessage(err)),
  });
  const withdraw = useMutation({
    mutationFn: (inviteId: string) => revokeInvite(workspace.id, inviteId),
    onSuccess: refresh,
    onError: (err) => toast.error(errorMessage(err)),
  });

  return (
    <Section
      title="Members"
      description={
        admin
          ? "Admins can do everything, including managing members. Write can change the workspace and run tests. Read can only look. There's always at least one admin."
          : undefined
      }
    >
      {members.isPending ? (
        <p className="text-muted-foreground text-sm">Loading…</p>
      ) : members.isError ? (
        <p className="text-muted-foreground text-sm">Couldn&apos;t load the members.</p>
      ) : (
        <ul className="divide-y rounded-xs border">
          {members.data.members.map((m) => (
            <li key={m.userId} className="flex items-center gap-3 px-3 py-2.5 text-sm">
              <span className="min-w-0 flex-1 truncate">
                {m.email}
                {m.you && <span className="text-muted-foreground"> (you)</span>}
              </span>
              {admin && !m.you ? (
                <RoleSelect
                  value={m.role}
                  disabled={changeRole.isPending}
                  onChange={(role) => changeRole.mutate({ userId: m.userId, role })}
                  label={`Role of ${m.email}`}
                />
              ) : (
                <RoleBadge role={m.role} />
              )}
              {admin && !m.you && (
                <Button
                  variant="ghost"
                  size="icon"
                  aria-label={`Remove ${m.email}`}
                  title="Remove from this workspace"
                  disabled={remove.isPending}
                  onClick={() => remove.mutate(m)}
                >
                  <UserMinus className="size-4" />
                </Button>
              )}
              {m.you && (
                <Button
                  variant="outline"
                  size="sm"
                  disabled={remove.isPending}
                  onClick={() => remove.mutate(m)}
                >
                  <LogOut className="size-3.5" /> Leave
                </Button>
              )}
            </li>
          ))}
          {members.data.invites.map((i) => (
            <li key={i.id} className="flex items-center gap-3 px-3 py-2.5 text-sm">
              <Mail className="text-muted-foreground size-4 shrink-0" />
              <span className="flex min-w-0 flex-1 flex-col">
                <span className="truncate">{i.email}</span>
                <span className="text-muted-foreground text-xs">
                  {inviteAccess(i.role)} · link expires{" "}
                  {new Date(i.expiresAtMs).toLocaleDateString()}
                </span>
              </span>
              <Button
                variant="outline"
                size="sm"
                disabled={withdraw.isPending}
                onClick={() => withdraw.mutate(i.id)}
              >
                Withdraw
              </Button>
            </li>
          ))}
        </ul>
      )}
    </Section>
  );
};

const RoleSelect = ({
  value,
  onChange,
  disabled,
  label,
}: {
  value: Role;
  onChange: (role: Role) => void;
  disabled?: boolean;
  label: string;
}) => (
  <Select
    value={value}
    items={MEMBER_ROLES}
    disabled={disabled}
    onValueChange={(v) => v && v !== value && onChange(v as Role)}
  >
    <SelectTrigger className="w-28" aria-label={label}>
      <SelectValue />
    </SelectTrigger>
    <SelectContent>
      {MEMBER_ROLES.map((r) => (
        <SelectItem key={r.value} value={r.value}>
          {r.label}
        </SelectItem>
      ))}
    </SelectContent>
  </Select>
);

const inviteSchema = z.object({
  email: z.string().trim().min(1, "Enter an email").email("Enter a valid email address"),
  role: z.enum(["admin", "write", "read"]),
});

const Invite = ({ workspace }: { workspace: WorkspaceInfo }) => {
  const queryClient = useQueryClient();
  const mail = useMe().data?.mail ?? false;
  const [created, setCreated] = useState<InviteCreated | null>(null);

  return (
    <Section
      title="Invite someone"
      description={
        mail
          ? "We'll email them a link. It works once, for 7 days, and only for that address."
          : "This engine can't send email, so send them the link yourself. It works once, for 7 days, and only for that address."
      }
    >
      <Form
        className="max-w-md"
        schema={inviteSchema}
        defaultValues={{ email: "", role: "write" as const }}
        toastOnInvalid={false}
        fields={{
          email: { type: "email", label: "Email address", placeholder: "teammate@example.com" },
          role: {
            type: "select",
            label: "Role",
            options: MEMBER_ROLES.map((r) => ({ value: r.value, label: `${r.label}: ${r.hint}` })),
          },
        }}
        onSubmit={async (req, form) => {
          try {
            const result = await inviteMember(workspace.id, req);
            setCreated(result);
            form.reset({ email: "", role: req.role });
            void queryClient.invalidateQueries({ queryKey: membersKey(workspace.id) });
            if (result.emailed) toast.success(`Invite sent to ${result.invite.email}.`);
          } catch (err) {
            toast.error(errorMessage(err));
          }
        }}
      >
        {({ field, isSubmitting }) => (
          <div className="flex flex-col gap-3">
            {field("email")}
            {field("role")}
            <Button type="submit" className="self-start" disabled={isSubmitting}>
              {isSubmitting ? "Inviting…" : "Invite"}
            </Button>
          </div>
        )}
      </Form>
      {created && <InviteLink created={created} />}
    </Section>
  );
};

const InviteLink = ({ created }: { created: InviteCreated }) => {
  const [copied, setCopied] = useState(false);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(created.link);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      toast.error("Couldn't copy. Select the link and copy it yourself.");
    }
  };
  return (
    <div className="bg-muted/40 flex max-w-xl flex-col gap-2 rounded-xs border p-3 text-sm">
      <p>
        {created.emailed
          ? `Emailed to ${created.invite.email}. You can also send them this link:`
          : `Send this link to ${created.invite.email}:`}
      </p>
      <div className="flex items-center gap-2">
        <code className="bg-background min-w-0 flex-1 truncate rounded-xs border px-2 py-1.5 text-xs">
          {created.link}
        </code>
        <Button variant="outline" size="sm" onClick={copy}>
          {copied ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}
          {copied ? "Copied" : "Copy"}
        </Button>
      </div>
    </div>
  );
};

const YourWorkspaces = () => {
  const me = useMe().data;
  const [creating, setCreating] = useState(false);
  if (!me) return null;
  return (
    <Section
      title="Your workspaces"
      description="Each has its own collections, environments, secrets and runs."
    >
      <ul className="divide-y rounded-xs border">
        {me.workspaces.map((w) => (
          <li key={w.id} className="flex items-center gap-3 px-3 py-2.5 text-sm">
            <span className="flex min-w-0 flex-1 flex-col">
              <span className="truncate font-medium">{w.name}</span>
              <span className="text-muted-foreground text-xs">
                {w.members === 1 ? "Just you" : `${w.members} members`}
                {w.role !== "admin" ? ` · admins: ${adminList(w.adminEmails)}` : ""}
              </span>
            </span>
            <RoleBadge role={w.role} />
            {w.id === me.workspaceId ? (
              <span className="text-muted-foreground w-16 text-center text-xs">Open</span>
            ) : (
              <Button
                variant="outline"
                size="sm"
                className="w-16"
                onClick={() => switchWorkspace(w.id)}
              >
                Switch
              </Button>
            )}
          </li>
        ))}
      </ul>
      {creating ? (
        <Form
          className="max-w-sm"
          schema={nameSchema}
          defaultValues={{ name: "" }}
          toastOnInvalid={false}
          fields={{
            name: {
              type: "text",
              label: "New workspace name",
              autoFocus: true,
              placeholder: "Payments team",
            },
          }}
          onSubmit={async ({ name }) => {
            try {
              const created = await createWorkspace(name);
              switchWorkspace(created.id);
            } catch (err) {
              toast.error(errorMessage(err));
            }
          }}
        >
          {({ field, isSubmitting }) => (
            <div className="flex flex-col gap-3">
              {field("name")}
              <div className="flex gap-2">
                <Button type="submit" disabled={isSubmitting}>
                  {isSubmitting ? "Creating…" : "Create and open"}
                </Button>
                <Button type="button" variant="outline" onClick={() => setCreating(false)}>
                  Cancel
                </Button>
              </div>
            </div>
          )}
        </Form>
      ) : (
        <Button variant="outline" className="self-start" onClick={() => setCreating(true)}>
          <Plus className="size-3.5" /> New workspace
        </Button>
      )}
    </Section>
  );
};

const DeleteWorkspace = ({ workspace }: { workspace: WorkspaceInfo }) => {
  const [armed, setArmed] = useState(false);
  const del = useMutation({
    mutationFn: () => deleteWorkspace(workspace.id),
    // The next page load lands in the default workspace.
    // eslint-disable-next-line @next/next/no-location-assign-relative-destination
    onSuccess: () => window.location.assign("/workspace"),
    onError: (err) => {
      setArmed(false);
      toast.error(errorMessage(err));
    },
  });
  // Disarm after a few seconds, so a stray second click later doesn't delete.
  useEffect(() => {
    if (!armed) return;
    const t = setTimeout(() => setArmed(false), 5000);
    return () => clearTimeout(t);
  }, [armed]);

  return (
    <Section
      title="Delete workspace"
      description={`Deletes "${workspace.name}" for everyone in it: collections, environments, secrets and run history. This can't be undone.`}
    >
      <Button
        variant="destructive"
        className="self-start"
        disabled={del.isPending}
        onClick={() => (armed ? del.mutate() : setArmed(true))}
      >
        <Trash2 className="size-3.5" />
        {del.isPending ? "Deleting…" : armed ? "Click again to delete" : "Delete workspace"}
      </Button>
    </Section>
  );
};
