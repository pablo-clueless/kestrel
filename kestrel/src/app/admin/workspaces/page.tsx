"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ChevronRight, Search, Trash2 } from "lucide-react";
import { Fragment, useMemo, useState } from "react";
import { toast } from "sonner";

import { AdminPage, Badge, QueryState } from "@/components/admin";
import { ConfirmButton } from "@/components/shared";
import { Input } from "@/components/ui/input";
import type { AdminWorkspace } from "@/types/engine/AdminWorkspace";
import {
  adminDeleteWorkspace,
  adminWorkspaceMembers,
  adminWorkspaces,
  errorMessage,
} from "@/lib/client";
import { bytes, date, int } from "@/lib/format";
import { cn } from "cn";

const ROLE_LABEL = { admin: "Admin", write: "Write", read: "Read" } as const;

const matches = (w: AdminWorkspace, q: string) =>
  w.name.toLowerCase().includes(q) ||
  w.adminEmails.some((e) => e.includes(q)) ||
  w.id.startsWith(q);

const Page = () => {
  const workspaces = useQuery({ queryKey: ["admin", "workspaces"], queryFn: adminWorkspaces });
  const [query, setQuery] = useState("");
  const [expanded, setExpanded] = useState<string | null>(null);
  const q = query.trim().toLowerCase();
  const shown = useMemo(
    () => (workspaces.data ?? []).filter((w) => !q || matches(w, q)),
    [workspaces.data, q],
  );
  const total = (workspaces.data ?? []).reduce((sum, w) => sum + w.sizeBytes, 0);

  return (
    <AdminPage
      title="Workspaces"
      description="Every workspace on this engine, newest first."
      actions={
        <div className="relative w-64">
          <Search className="text-muted-foreground pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2" />
          <Input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search by name, admin or id"
            aria-label="Search workspaces"
            className="pl-8"
          />
        </div>
      }
    >
      <QueryState query={workspaces}>
        {() => (
          <div className="bg-card overflow-x-auto rounded-xs border">
            <table className="w-full text-sm">
              <thead className="text-muted-foreground border-b text-left text-xs">
                <tr>
                  <th className="w-8 py-2.5 pl-4" aria-label="Members" />
                  <th className="px-4 py-2.5 font-medium">Workspace</th>
                  <th className="px-4 py-2.5 font-medium">Admins</th>
                  <th className="px-4 py-2.5 text-right font-medium">Members</th>
                  <th className="px-4 py-2.5 text-right font-medium">Size</th>
                  <th className="px-4 py-2.5 font-medium">Created</th>
                  <th className="px-4 py-2.5" aria-label="Actions" />
                </tr>
              </thead>
              <tbody className="divide-y">
                {shown.map((w) => {
                  const open = expanded === w.id;
                  return (
                    <Fragment key={w.id}>
                      <tr>
                        <td className="py-2.5 pl-4">
                          <button
                            type="button"
                            onClick={() => setExpanded(open ? null : w.id)}
                            aria-expanded={open}
                            aria-label={`${open ? "Hide" : "Show"} members of ${w.name}`}
                            className="text-muted-foreground hover:text-foreground"
                          >
                            <ChevronRight
                              className={cn("size-4 transition-transform", open && "rotate-90")}
                            />
                          </button>
                        </td>
                        <td className="px-4 py-2.5">
                          <span className="font-medium">{w.name}</span>
                          <span className="text-muted-foreground block font-mono text-[11px]">
                            {w.id}
                          </span>
                        </td>
                        <td className="text-muted-foreground px-4 py-2.5 text-xs">
                          {w.adminEmails.length ? w.adminEmails.join(", ") : "–"}
                        </td>
                        <td className="px-4 py-2.5 text-right tabular-nums">{int(w.members)}</td>
                        <td className="px-4 py-2.5 text-right whitespace-nowrap tabular-nums">
                          {bytes(w.sizeBytes)}
                        </td>
                        <td className="text-muted-foreground px-4 py-2.5 text-xs whitespace-nowrap">
                          {date(w.createdAtMs)}
                        </td>
                        <td className="px-4 py-2.5 text-right">
                          <DeleteWorkspace workspace={w} />
                        </td>
                      </tr>
                      {open && (
                        <tr className="bg-muted/30">
                          <td />
                          <td colSpan={6} className="px-4 py-3">
                            <Members id={w.id} />
                          </td>
                        </tr>
                      )}
                    </Fragment>
                  );
                })}
              </tbody>
            </table>
            {shown.length === 0 && (
              <p className="text-muted-foreground px-4 py-8 text-center text-sm">
                {q ? `No workspaces match “${query.trim()}”.` : "No workspaces yet."}
              </p>
            )}
            <p className="text-muted-foreground border-t px-4 py-2 text-xs">
              {q
                ? `${int(shown.length)} of ${int(workspaces.data!.length)} workspaces`
                : `${int(workspaces.data!.length)} workspaces`}
              {" · "}
              {bytes(total)} in all
            </p>
          </div>
        )}
      </QueryState>
    </AdminPage>
  );
};

const Members = ({ id }: { id: string }) => {
  const members = useQuery({
    queryKey: ["admin", "workspaces", id, "members"],
    queryFn: () => adminWorkspaceMembers(id),
  });
  if (members.isPending) return <p className="text-muted-foreground text-xs">Loading members…</p>;
  if (members.isError)
    return <p className="text-destructive text-xs">{errorMessage(members.error)}</p>;
  if (members.data.length === 0) {
    return <p className="text-muted-foreground text-xs">Nobody is a member of this workspace.</p>;
  }
  return (
    <ul className="flex flex-col gap-1.5">
      {members.data.map((m) => (
        <li key={m.userId} className="flex items-center justify-between gap-3 text-xs">
          <span className="flex items-center gap-1.5">
            <span className="font-medium">{m.name ?? m.email}</span>
            {m.name && <span className="text-muted-foreground">{m.email}</span>}
            {m.you && <Badge tone="primary">You</Badge>}
          </span>
          <span className="text-muted-foreground">{ROLE_LABEL[m.role]}</span>
        </li>
      ))}
    </ul>
  );
};

const DeleteWorkspace = ({ workspace }: { workspace: AdminWorkspace }) => {
  const queryClient = useQueryClient();
  const remove = useMutation({
    mutationFn: () => adminDeleteWorkspace(workspace.id),
    onSuccess: () => {
      toast.success(`Deleted “${workspace.name}”.`);
      void queryClient.invalidateQueries({ queryKey: ["admin"] });
    },
    onError: (err) => toast.error(errorMessage(err)),
  });
  return (
    <ConfirmButton
      variant="ghost"
      size="sm"
      className="text-destructive hover:text-destructive"
      title={`Delete “${workspace.name}” and everything in it, for everyone in it`}
      pending={remove.isPending}
      pendingLabel="Deleting…"
      confirmLabel="Click again to delete"
      onConfirm={() => remove.mutate()}
    >
      <Trash2 className="size-3.5" /> Delete
    </ConfirmButton>
  );
};

export default Page;
