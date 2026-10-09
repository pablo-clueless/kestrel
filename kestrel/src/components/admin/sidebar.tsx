"use client";

import { ArrowLeft } from "lucide-react";
import { usePathname } from "next/navigation";
import Image from "next/image";
import Link from "next/link";

import { WORKSPACE_PATH } from "@/hooks/use-to-workspace";
import { useLayoutStore } from "@/stores/layout-store";
import { SIDEBAR_CONFIG } from "@/config/sidebar";
import { cn } from "cn";

const ADMIN_HOME = "/admin/overview";
const IMAGE = "/assets/logo.png";

const link = "flex items-center gap-x-2 rounded-xs px-2 py-2 text-sm transition-colors";

export const Sidebar = () => {
  const open = useLayoutStore((s) => s.sidebarOpen);
  const pathname = usePathname();

  return (
    <aside
      inert={!open}
      aria-hidden={!open}
      className={cn(
        "bg-card h-full shrink-0 overflow-hidden border-r transition-[width,border-color] duration-300 ease-[cubic-bezier(0.32,0.72,0,1)] motion-reduce:transition-none",
        open ? "w-68" : "w-0 border-r-transparent",
      )}
    >
      <div
        className={cn(
          "flex h-full w-68 flex-col transition-[opacity,translate] duration-300 ease-[cubic-bezier(0.32,0.72,0,1)] motion-reduce:transition-none",
          open ? "translate-x-0 opacity-100" : "-translate-x-4 opacity-0",
        )}
      >
        <div className="flex h-14 shrink-0 items-center gap-2 border-b px-4 font-semibold">
          <Link
            href={ADMIN_HOME}
            aria-label="Kestrel admin: go to the overview"
            className="relative aspect-[4.2/1] w-1/3"
          >
            <Image alt="Kestrel" fill src={IMAGE} />
          </Link>
          <span className="bg-primary/15 text-primary rounded-full px-2 py-0.5 text-[11px] font-medium">
            Admin
          </span>
        </div>
        <nav aria-label="Admin" className="flex min-h-0 flex-1 flex-col justify-between">
          <div className="space-y-5 overflow-y-auto p-4">
            {SIDEBAR_CONFIG.map((group) => (
              <div className="space-y-1" key={group.label}>
                <p className="text-muted-foreground px-2 text-xs uppercase">{group.label}</p>
                <div className="flex flex-col gap-0.5">
                  {group.routes.map((route) => {
                    const Icon = route.icon;
                    if (route.disabled) {
                      return (
                        <span
                          key={route.href}
                          aria-disabled="true"
                          title="Coming later"
                          className={cn(
                            link,
                            "text-muted-foreground cursor-not-allowed opacity-50",
                          )}
                        >
                          <Icon className="size-4" />
                          {route.label}
                        </span>
                      );
                    }
                    const active = pathname === route.href || pathname.startsWith(`${route.href}/`);
                    return (
                      <Link
                        key={route.href}
                        href={route.href}
                        aria-current={active ? "page" : undefined}
                        className={cn(
                          link,
                          active
                            ? "bg-muted text-foreground font-medium"
                            : "text-muted-foreground hover:text-foreground hover:bg-muted/60",
                        )}
                      >
                        <Icon className="size-4" />
                        {route.label}
                      </Link>
                    );
                  })}
                </div>
              </div>
            ))}
          </div>
          <div className="border-t p-4">
            <Link
              href={WORKSPACE_PATH}
              className={cn(link, "text-muted-foreground hover:text-foreground hover:bg-muted/60")}
            >
              <ArrowLeft className="size-4" /> Back to workspace
            </Link>
          </div>
        </nav>
      </div>
    </aside>
  );
};
