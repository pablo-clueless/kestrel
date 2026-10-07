"use client";

import { AnimatePresence, motion } from "framer-motion";
import { useEffect, useRef, useState } from "react";
import {
  ChartNoAxesGantt,
  ChevronRight,
  ChevronsDownUp,
  FileUp,
  LucideIcon,
  Plus,
  Settings2,
  Trash,
  Trash2,
  Variable,
} from "lucide-react";

import { activeCollectionOf, useWorkspaceStore } from "@/stores/workspace-store";
import { InlineNameForm } from "@/components/shared/inline-name-form";
import { AddPairForm } from "@/components/shared/add-pair-form";
import type { Collection } from "@/types/engine/Collection";
import { useLayoutStore } from "@/stores/layout-store";
import { Button } from "@/components/ui/button";
import { EndpointList } from "./endpoint-list";
import { KeyValueEditor } from "./key-value-editor";
import { ImportDialog } from "./import-dialog";
import { useValues } from "@/hooks/use-values";
import { useWorkspaceSync } from "@/hooks/use-workspace-sync";
import { useCanEdit } from "@/hooks/use-me";
import { Editable } from "../shared/editable";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";
import { Label } from "./fields";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

/** Height + fade for sections that expand/collapse in the sidebar. */
const collapse = {
  initial: { height: 0, opacity: 0 },
  animate: { height: "auto", opacity: 1 },
  exit: { height: 0, opacity: 0 },
  transition: { duration: 0.25, ease: [0.32, 0.72, 0, 1] },
} as const;

/** Row actions that fade in on hover/focus instead of popping in (keeps row layout stable). */
const hoverAction =
  "text-muted-foreground pointer-events-none opacity-0 transition-[opacity,color] duration-150 group-hover:pointer-events-auto group-hover:opacity-100 focus-visible:pointer-events-auto focus-visible:opacity-100";

type SettingsTab = "variables" | "headers";

const TABS: { id: SettingsTab; label: string; icon: LucideIcon }[] = [
  { id: "variables", label: "Variables", icon: Variable },
  { id: "headers", label: "Headers", icon: ChartNoAxesGantt },
];

/** Sidebar: collections of endpoints. Clicking a collection opens or closes it, so any number can
 * be open, none included; it doesn't select anything. The active collection (in bold) is the one
 * whose endpoint you last selected or added. Also loads the workspace. */
export const CollectionList = () => {
  const { workspace, loadError, load, addCollection } = useWorkspaceStore();
  const { values, set } = useValues({
    initialValue: {
      // Whether the inline "new collection" name field is open.
      adding: false,
      // Which dialog is open.
      editing: null as Collection | null,
      deleting: null as Collection | null,
      importing: false,
    },
  });
  const { adding, editing, deleting, importing } = values;
  const active = activeCollectionOf(workspace);
  const { expandedCollections, expandCollection, collapseCollection, collapseAllCollections } =
    useLayoutStore();

  useEffect(() => {
    void load();
  }, [load]);
  useWorkspaceSync();
  const canEdit = useCanEdit();

  // A collection you've just made or imported becomes active and opens, so you can see it. Nothing
  // else opens by itself: not the active collection after a reload, nor the one that becomes
  // active when you delete another.
  const activeId = active?.id;
  const ids = workspace?.collections.map((c) => c.id).join(",");
  const known = useRef<Set<string> | null>(null);
  useEffect(() => {
    if (ids === undefined) return;
    const current = new Set(ids ? ids.split(",") : []);
    if (known.current && activeId && !known.current.has(activeId)) expandCollection(activeId);
    known.current = current;
  }, [ids, activeId, expandCollection]);

  if (loadError) {
    return (
      <motion.div className="flex flex-col gap-2 text-sm">
        <p className="text-red-600">{loadError}</p>
        <button className="text-primary self-start" onClick={() => void load()}>
          Retry
        </button>
      </motion.div>
    );
  }

  return (
    <motion.div className="bg-background flex h-full flex-col gap-3 p-3">
      <motion.div className="flex items-center justify-between">
        <span className="text-muted-foreground text-xs uppercase">Collections</span>
        <motion.div className="flex items-center gap-2">
          <button
            className="text-muted-foreground hover:text-primary disabled:pointer-events-none disabled:opacity-40"
            onClick={collapseAllCollections}
            disabled={expandedCollections.length === 0}
            aria-label="Collapse all collections"
            title="Collapse all"
          >
            <ChevronsDownUp className="size-4" />
          </button>
          {canEdit && (
            <button
              className="text-muted-foreground hover:text-primary"
              onClick={() => set("importing", true)}
              disabled={!workspace}
              aria-label="Import API spec"
              title="Import OpenAPI / Swagger"
            >
              <FileUp className="size-4" />
            </button>
          )}
          {canEdit && (
            <button
              className="text-muted-foreground hover:text-primary"
              onClick={() => set("adding", true)}
              disabled={!workspace}
              aria-label="New collection"
              title="New empty collection"
            >
              <Plus className="size-4" />
            </button>
          )}
        </motion.div>
      </motion.div>
      <AnimatePresence initial={false}>
        {adding && (
          <motion.div {...collapse} className="overflow-hidden">
            <InlineNameForm
              placeholder="Collection name"
              onSubmit={(name) => {
                addCollection(name);
                set("adding", false);
              }}
              onCancel={() => set("adding", false)}
            />
          </motion.div>
        )}
      </AnimatePresence>
      <nav className="-mx-2 flex-1 overflow-y-auto">
        {workspace?.collections.map((c) => {
          const isActive = c.id === active?.id;
          const isOpen = expandedCollections.includes(c.id);
          return (
            <motion.div key={c.id} className="mb-1">
              <motion.div
                className={cn(
                  "group flex items-center gap-1.5 rounded-xs px-2 py-1.5 text-sm transition-colors duration-150",
                  isActive ? "font-medium" : "hover:bg-muted",
                )}
              >
                <button
                  className="flex min-w-0 flex-1 items-center gap-1.5 text-left"
                  onClick={() => (isOpen ? collapseCollection(c.id) : expandCollection(c.id))}
                  aria-expanded={isOpen}
                >
                  <ChevronRight
                    className={cn(
                      "size-4 shrink-0 transition-[rotate,color] duration-200 ease-out motion-reduce:transition-none",
                      isOpen ? "rotate-90" : "text-muted-foreground",
                    )}
                  />
                  <span className="truncate">{c.name}</span>
                  <span className="text-muted-foreground text-xs font-normal">
                    {c.endpoints.length}
                  </span>
                </button>
                <button
                  className={cn(hoverAction, "hover:text-primary")}
                  onClick={() => set("editing", c)}
                  aria-label={`Settings for ${c.name}`}
                >
                  <Settings2 className="size-3.5" />
                </button>
                {canEdit && (
                  <button
                    className={cn(hoverAction, "hover:text-red-600")}
                    onClick={() => set("deleting", c)}
                    aria-label={`Delete ${c.name}`}
                  >
                    <Trash2 className="size-3.5" />
                  </button>
                )}
              </motion.div>
              <AnimatePresence initial={false}>
                {isOpen && (
                  <motion.div key="endpoints" {...collapse} className="overflow-hidden">
                    <EndpointList collection={c} />
                  </motion.div>
                )}
              </AnimatePresence>
            </motion.div>
          );
        })}
        {workspace && workspace.collections.length === 0 && (
          <p className="text-muted-foreground px-2 text-sm">
            No collections yet. Import an OpenAPI or Swagger spec, or press + to start an empty one.
          </p>
        )}
      </nav>
      <CollectionSettingsDialog collection={editing} onClose={() => set("editing", null)} />
      <DeleteCollectionDialog collection={deleting} onClose={() => set("deleting", null)} />
      <ImportDialog open={importing} onClose={() => set("importing", false)} />
    </motion.div>
  );
};

/** Rename, variables that act as defaults for this collection's endpoints, and headers sent with
 * all of them. */
const CollectionSettingsDialog = ({
  collection,
  onClose,
}: {
  collection: Collection | null;
  onClose: () => void;
}) => {
  const { renameCollection, setCollectionVar, setCollectionHeaders } = useWorkspaceStore();
  const [tab, setTab] = useState<SettingsTab>("variables");

  // Read live values from the store so edits show immediately.
  const live = useWorkspaceStore(
    (s) => s.workspace?.collections.find((c) => c.id === collection?.id) ?? null,
  );

  return (
    <Dialog open={live !== null} onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="sm:max-w-150">
        <DialogHeader>
          <DialogTitle>Collection settings</DialogTitle>
          <DialogDescription>
            Settings shared by every endpoint in this collection.
          </DialogDescription>
        </DialogHeader>
        {live && (
          <motion.div className="flex flex-col gap-4 text-sm">
            <Editable>
              <label className="flex flex-col gap-1.5">
                <Label>Name</Label>
                <Input
                  value={live.name}
                  onChange={(e) => renameCollection(live.id, e.target.value)}
                />
              </label>
            </Editable>
            <motion.div className="">
              <div role="tablist" aria-label="Collection settings tabs" className="flex">
                {TABS.map(({ id, label, icon: Icon }) => (
                  <button
                    key={id}
                    role="tab"
                    id={`tab-${id}`}
                    aria-selected={tab === id}
                    aria-controls={`tabpanel-${id}`}
                    tabIndex={tab === id ? 0 : -1}
                    onClick={() => setTab(id)}
                    className={cn(
                      "flex flex-1 items-center justify-center gap-1.5 px-2 py-2.5 text-xs transition-colors duration-150",
                      tab === id
                        ? "bg-background text-foreground font-medium"
                        : "text-muted-foreground hover:text-foreground hover:bg-muted/60 border-b",
                    )}
                  >
                    <Icon className="size-3.5" />
                    {label}
                  </button>
                ))}
              </div>
              <Editable>
                {tab === "variables" && (
                  <div
                    role="tabpanel"
                    id="tabpanel-variables"
                    aria-labelledby="tab-variables"
                    className="bg-background flex flex-col gap-1.5 p-2"
                  >
                    <p className="text-muted-foreground text-xs">
                      Defaults for <code>{"{{…}}"}</code> in this collection&apos;s endpoints. The
                      active environment&apos;s variables and secrets override them.
                    </p>
                    {Object.entries(live.vars).map(([k, v]) => (
                      <motion.div key={k} className="flex items-center gap-4">
                        <span className="w-24 shrink-0 truncate font-mono text-xs" title={k}>
                          {k}
                        </span>
                        <Input
                          className="flex-1 font-mono"
                          value={v}
                          onChange={(e) => setCollectionVar(live.id, k, e.target.value)}
                        />
                        <button
                          className="text-muted-foreground hover:text-red-500"
                          onClick={() => setCollectionVar(live.id, k, null)}
                          aria-label={`Remove ${k}`}
                        >
                          <Trash className="size-4" />
                        </button>
                      </motion.div>
                    ))}
                    <AddPairForm
                      noun="Variable"
                      keyPlaceholder="base"
                      valuePlaceholder="https://api.example.com"
                      keyClassName="w-24"
                      onAdd={(key, value) => setCollectionVar(live.id, key, value)}
                    />
                  </div>
                )}
                {tab === "headers" && (
                  <div
                    role="tabpanel"
                    id="tabpanel-headers"
                    aria-labelledby="tab-headers"
                    className="bg-background flex flex-col gap-1.5 p-2"
                  >
                    <p className="text-muted-foreground text-xs">
                      Sent with every endpoint in this collection, on Send and in runs. Values may
                      use <code>{"{{variables}}"}</code>. An endpoint&apos;s own header or auth with
                      the same name wins.
                    </p>
                    <KeyValueEditor
                      rows={live.headers ?? []}
                      onChange={(headers) => setCollectionHeaders(live.id, headers)}
                      keyPlaceholder="X-Tenant"
                    />
                  </div>
                )}
              </Editable>
            </motion.div>
          </motion.div>
        )}
        <DialogFooter>
          <Button onClick={onClose}>Done</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
};

const DeleteCollectionDialog = ({
  collection,
  onClose,
}: {
  collection: Collection | null;
  onClose: () => void;
}) => {
  const removeCollection = useWorkspaceStore((s) => s.removeCollection);
  const count = collection?.endpoints.length ?? 0;
  return (
    <Dialog open={collection !== null} onOpenChange={(open) => !open && onClose()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Delete {collection?.name}?</DialogTitle>
          <DialogDescription>
            This removes the collection and its {count} endpoint{count === 1 ? "" : "s"}.
          </DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="destructive"
            onClick={() => {
              if (collection) removeCollection(collection.id);
              onClose();
            }}
          >
            Delete
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
};
