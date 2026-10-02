"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import { Reorder } from "framer-motion";
import { Plus, X } from "lucide-react";

import { MethodBadge } from "@/components/shared/method-badge";
import { useWorkspaceStore } from "@/stores/workspace-store";
import type { Endpoint } from "@/types/engine/Endpoint";
import { useSendStore } from "@/stores/send-store";
import { CircleLoader } from "../shared";
import { cn } from "@/lib/utils";

/** A tab's full width; keep in step with `basis-56` on the tab. */
const TAB_REM = 14;

/**
 * Open requests as browser-like tabs. Each tab wants 14rem and they all shrink evenly as more open,
 * down to 2.5rem, after which the strip scrolls. Narrow tabs drop the name, then the close button
 * (the active tab keeps its close button instead of the badge). Drag to reorder, middle-click to close.
 * As in a browser, closing with the mouse holds tab widths until the pointer leaves the strip, so the
 * next close button lands under the cursor.
 */
export const RequestTabs = () => {
  const workspace = useWorkspaceStore((s) => s.workspace);
  const drafts = useWorkspaceStore((s) => s.drafts);
  const openIds = useWorkspaceStore((s) => s.openIds);
  const selectedId = useWorkspaceStore((s) => s.selectedId);
  const { select, closeTab, reorderTabs, newTab } = useWorkspaceStore.getState();

  const endpoints = useMemo(
    () =>
      new Map([
        ...(workspace?.collections.flatMap((c) => c.endpoints.map((e) => [e.id, e] as const)) ??
          []),
        ...Object.entries(drafts),
      ]),
    [workspace, drafts],
  );

  const tabIds = openIds.filter((id) => endpoints.has(id));

  const stripRef = useRef<HTMLDivElement>(null);
  const tabRefs = useRef(new Map<string, HTMLElement>());

  const [frozenWidth, setFrozenWidth] = useState<number | null>(null);
  const closeWithMouse = (id: string) => {
    const tab = tabRefs.current.get(id);
    const list = tab?.parentElement;
    // Only hold widths while every tab fits: once the strip scrolls, tabs are at their minimum and
    // holding that would keep them squashed after closing.
    const fits = list && list.scrollWidth <= list.clientWidth;
    setFrozenWidth(tab && fits ? tab.getBoundingClientRect().width : null);
    closeTab(id);
  };

  // Opening a tab lets widths refit (e.g. + clicked right after closing some).
  const tabCount = tabIds.length;
  const prevCount = useRef(tabCount);
  useEffect(() => {
    if (tabCount > prevCount.current) setFrozenWidth(null);
    prevCount.current = tabCount;
  }, [tabCount]);

  // Release held widths once the pointer is outside the strip. Checked on the document as well as
  // pointerleave, which can be skipped when the element under the pointer is removed.
  useEffect(() => {
    if (frozenWidth === null) return;
    const release = (e: PointerEvent) => {
      if (!(e.target instanceof Node) || !stripRef.current?.contains(e.target)) {
        setFrozenWidth(null);
      }
    };
    const releaseAll = () => setFrozenWidth(null);
    document.addEventListener("pointermove", release);
    document.addEventListener("pointerdown", release);
    window.addEventListener("blur", releaseAll);
    return () => {
      document.removeEventListener("pointermove", release);
      document.removeEventListener("pointerdown", release);
      window.removeEventListener("blur", releaseAll);
    };
  }, [frozenWidth]);

  // Keep the active tab visible when the strip scrolls.
  useEffect(() => {
    if (selectedId) {
      tabRefs.current.get(selectedId)?.scrollIntoView({ block: "nearest", inline: "nearest" });
    }
  }, [selectedId, openIds.length]);

  const onKeyDown = (e: React.KeyboardEvent, id: string) => {
    const i = openIds.indexOf(id);
    const next =
      e.key === "ArrowRight"
        ? openIds[i + 1]
        : e.key === "ArrowLeft"
          ? openIds[i - 1]
          : e.key === "Home"
            ? openIds[0]
            : e.key === "End"
              ? openIds.at(-1)
              : undefined;
    if (next) {
      e.preventDefault();
      select(next);
      tabRefs.current.get(next)?.focus();
    } else if (e.key === "Delete") {
      e.preventDefault();
      closeTab(id);
    }
  };

  return (
    <div
      ref={stripRef}
      className="bg-card flex h-9 shrink-0 items-stretch"
      onPointerLeave={() => setFrozenWidth(null)}
    >
      <Reorder.Group
        as="div"
        axis="x"
        values={openIds}
        onReorder={reorderTabs}
        layoutScroll
        role="tablist"
        aria-label="Open requests"
        // Wants every tab at full width, then shrinks to the space left beside the + button.
        style={{ flexBasis: `${tabIds.length * TAB_REM}rem` }}
        className="flex min-w-0 shrink grow-0 items-stretch overflow-x-auto"
      >
        {tabIds.map((id) => {
          const endpoint = endpoints.get(id)!;
          return (
            <RequestTab
              key={id}
              endpoint={endpoint}
              active={id === selectedId}
              draft={id in drafts}
              onSelect={() => select(id)}
              width={frozenWidth}
              onClose={() => closeWithMouse(id)}
              onKeyDown={(e) => onKeyDown(e, id)}
              ref={(el) => {
                if (el) tabRefs.current.set(id, el);
                else tabRefs.current.delete(id);
              }}
            />
          );
        })}
      </Reorder.Group>
      <button
        className="text-muted-foreground hover:text-foreground hover:bg-muted/60 flex w-9 shrink-0 items-center justify-center transition-colors"
        onClick={newTab}
        aria-label="New request"
        title="New request"
      >
        <Plus className="size-4" />
      </button>
    </div>
  );
};

const RequestTab = ({
  endpoint,
  active,
  draft,
  width,
  onSelect,
  onClose,
  onKeyDown,
  ref,
}: {
  endpoint: Endpoint;
  active: boolean;
  /** Unsaved: shown in italics. */
  draft: boolean;
  /** Fixed width in px while closing tabs with the mouse; null to fit the strip. */
  width: number | null;
  onSelect: () => void;
  onClose: () => void;
  onKeyDown: (e: React.KeyboardEvent) => void;
  ref: (el: HTMLDivElement | null) => void;
}) => {
  const pending = useSendStore((s) => s.byId[endpoint.id]?.pending ?? false);
  const label = endpoint.name || endpoint.url;

  return (
    <Reorder.Item
      as="div"
      ref={ref}
      value={endpoint.id}
      layout="position"
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      transition={{ duration: 0.15 }}
      role="tab"
      aria-selected={active}
      tabIndex={active ? 0 : -1}
      title={`${endpoint.method} ${label}`}
      onPointerDown={(e) => e.button === 0 && onSelect()}
      // Middle-click closes; preventing mousedown stops the browser's autoscroll cursor.
      onMouseDown={(e) => e.button === 1 && e.preventDefault()}
      onAuxClick={(e) => e.button === 1 && onClose()}
      onKeyDown={onKeyDown}
      style={width === null ? undefined : { flex: `0 0 ${width}px` }}
      className={cn(
        "group @container relative flex min-w-10 shrink grow-0 basis-56 cursor-default items-center gap-1.5 border-r px-2 text-xs outline-none select-none",
        "focus-visible:ring-ring/50 focus-visible:ring-2 focus-visible:ring-inset",
        active
          ? "bg-background text-foreground -mb-px font-medium"
          : "text-muted-foreground hover:bg-muted/60 hover:text-foreground bg-card",
      )}
    >
      {active && <span className="bg-primary absolute inset-x-0 top-0 h-0.5" />}
      <span className={cn("relative flex shrink-0", active && "@max-[4rem]:hidden")}>
        <MethodBadge method={endpoint.method} short />
        {pending && (
          <span className="bg-primary ring-card absolute -top-0.5 -right-0.5 size-1.5 animate-pulse rounded-full ring-2" />
        )}
      </span>
      <span className={cn("min-w-0 flex-1 truncate @max-[6rem]:hidden", draft && "italic")}>
        {label}
      </span>
      {pending ? (
        <CircleLoader radius={10} />
      ) : (
        <button
          className={cn(
            "text-muted-foreground hover:text-foreground hover:bg-muted ml-auto flex size-4 shrink-0 items-center justify-center transition-opacity",
            active
              ? "@max-[4rem]:mx-auto"
              : "opacity-0 group-hover:opacity-100 focus-visible:opacity-100 @max-[6rem]:hidden",
          )}
          onPointerDown={(e) => e.stopPropagation()}
          onClick={onClose}
          tabIndex={-1}
          aria-label={`Close ${label}`}
        >
          <X className="size-3" />
        </button>
      )}
    </Reorder.Item>
  );
};
