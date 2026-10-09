"use client";

import { useQuery } from "@tanstack/react-query";
import { RefreshCw } from "lucide-react";
import Link from "next/link";

import { AdminPage, Badge, QueryState } from "@/components/admin";
import { Card, StatGrid, StatTile } from "@/components/shared";
import { Button } from "@/components/ui/button";
import { adminOverview } from "@/lib/client";
import { int, relative } from "@/lib/format";

/** A share of the total, for a tile's tooltip: "12 of 40 (30%)". */
const share = (part: number, total: number) =>
  `${int(part)} of ${int(total)} (${total ? Math.round((part / total) * 100) : 0}%)`;

const Page = () => {
  const overview = useQuery({ queryKey: ["admin", "overview"], queryFn: adminOverview });

  return (
    <AdminPage
      title="Overview"
      description="Everyone and everything on this engine."
      actions={
        <Button
          variant="outline"
          size="sm"
          onClick={() => void overview.refetch()}
          disabled={overview.isFetching}
        >
          <RefreshCw className="size-3.5" /> {overview.isFetching ? "Refreshing…" : "Refresh"}
        </Button>
      }
    >
      <QueryState query={overview}>
        {() => {
          const o = overview.data!;
          return (
            <>
              <Card title="Accounts">
                <StatGrid cols={4}>
                  <StatTile label="Users" value={int(o.users)} />
                  <StatTile
                    label="Email verified"
                    value={int(o.verifiedUsers)}
                    info={share(o.verifiedUsers, o.users)}
                  />
                  <StatTile
                    label="Two-factor on"
                    value={int(o.twoFactorUsers)}
                    info={share(o.twoFactorUsers, o.users)}
                  />
                  <StatTile label="New in the last 7 days" value={int(o.signupsLast7d)} />
                  <StatTile
                    label="Disabled"
                    value={int(o.disabledUsers)}
                    tone={o.disabledUsers ? "bad" : "default"}
                  />
                  <StatTile
                    label="Signed-in sessions"
                    value={int(o.liveSessions)}
                    info="Sessions that haven't expired, across every device"
                  />
                </StatGrid>
              </Card>
              <Card title="Workspaces">
                <StatGrid cols={4}>
                  <StatTile label="Workspaces" value={int(o.workspaces)} />
                  <StatTile
                    label="Shared"
                    value={int(o.sharedWorkspaces)}
                    info="Workspaces with more than one member"
                  />
                  <StatTile
                    label="Tests running now"
                    value={int(o.runningTests)}
                    tone={o.runningTests ? "good" : "default"}
                  />
                </StatGrid>
              </Card>
              <Card
                title="Newest accounts"
                actions={
                  <Link
                    href="/admin/users"
                    className="text-muted-foreground hover:text-foreground text-sm"
                  >
                    All users →
                  </Link>
                }
                bodyClassName="px-0 pb-0"
              >
                {o.recentSignups.length === 0 ? (
                  <p className="text-muted-foreground px-5 pb-5 text-sm">No accounts yet.</p>
                ) : (
                  <ul className="divide-y border-t">
                    {o.recentSignups.map((u) => (
                      <li
                        key={u.id}
                        className="flex items-center justify-between gap-3 px-5 py-2.5"
                      >
                        <div className="flex min-w-0 flex-col">
                          <span className="flex items-center gap-2 truncate text-sm font-medium">
                            {u.name ?? u.email}
                            {u.you && <Badge tone="primary">You</Badge>}
                            {!u.emailVerified && <Badge tone="warning">Unverified</Badge>}
                            {u.disabled && <Badge tone="destructive">Disabled</Badge>}
                          </span>
                          {u.name && (
                            <span className="text-muted-foreground truncate text-xs">
                              {u.email}
                            </span>
                          )}
                        </div>
                        <span className="text-muted-foreground shrink-0 text-xs">
                          joined {relative(u.createdAtMs)}
                        </span>
                      </li>
                    ))}
                  </ul>
                )}
              </Card>
            </>
          );
        }}
      </QueryState>
    </AdminPage>
  );
};

export default Page;
