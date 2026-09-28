"use client";

import { ChevronDown, ChevronRight, FileUp, Plus, Settings2, Trash2, X } from "lucide-react";
import { useEffect } from "react";

import { activeCollectionOf, useWorkspaceStore } from "@/stores/workspace-store";
import type { Collection } from "@/types/engine/Collection";
import { Button } from "@/components/ui/button";
import { EndpointList } from "./endpoint-list";
import { ImportDialog } from "./import-dialog";
import { Input } from "@/components/ui/input";
import { useValues } from "@/hooks/use-values";
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

/** Sidebar: collections of endpoints. The active one is expanded; clicking another switches to it.
 * Also loads the workspace. */
export const CollectionList = () => {
  const { workspace, loadError, load, setActiveCollection, addCollection } = useWorkspaceStore();
  const { values, set, patch } = useValues({
    initialValue: {
      // Inline "new collection" name field.
      adding: false,
      newName: "",
      // Which dialog is open.
      editing: null as Collection | null,
      deleting: null as Collection | null,
      importing: false,
    },
  });
  const { adding, newName, editing, deleting, importing } = values;
  const active = activeCollectionOf(workspace);

  useEffect(() => {
    void load();
  }, [load]);

  if (loadError) {
    return (
      <div className="flex flex-col gap-2 text-sm">
        <p className="text-red-600">{loadError}</p>
        <button className="text-primary self-start" onClick={() => void load()}>
          Retry
        </button>
      </div>
    );
  }

  const create = () => {
    const name = newName.trim();
    if (name) addCollection(name);
    patch({ newName: "", adding: false });
  };

  return (
    <div className="flex h-full flex-col gap-3">
      <div className="flex items-center justify-between">
        <span className="text-muted-foreground text-xs uppercase">Collections</span>
        <div className="flex items-center gap-2">
          <button
            className="text-muted-foreground hover:text-primary"
            onClick={() => set("importing", true)}
            disabled={!workspace}
            aria-label="Import API spec"
            title="Import OpenAPI / Swagger"
          >
            <FileUp className="size-4" />
          </button>
          <button
            className="text-muted-foreground hover:text-primary"
            onClick={() => set("adding", true)}
            disabled={!workspace}
            aria-label="New collection"
            title="New empty collection"
          >
            <Plus className="size-4" />
          </button>
        </div>
      </div>
      {adding && (
        <form
          onSubmit={(e) => {
            e.preventDefault();
            create();
          }}
        >
          <Input
            autoFocus
            placeholder="Collection name"
            value={newName}
            onChange={(e) => set("newName", e.target.value)}
            onBlur={create}
            onKeyDown={(e) => e.key === "Escape" && set("adding", false)}
          />
        </form>
      )}
      <nav className="-mx-2 flex-1 overflow-y-auto">
        {workspace?.collections.map((c) => {
          const isActive = c.id === active?.id;
          return (
            <div key={c.id} className="mb-1">
              <div
                className={cn(
                  "group flex items-center gap-1.5 rounded-xs px-2 py-1.5 text-sm",
                  isActive ? "font-medium" : "hover:bg-muted/60",
                )}
              >
                <button
                  className="flex min-w-0 flex-1 items-center gap-1.5 text-left"
                  onClick={() => setActiveCollection(c.id)}
                  aria-expanded={isActive}
                >
                  {isActive ? (
                    <ChevronDown className="size-4 shrink-0" />
                  ) : (
                    <ChevronRight className="text-muted-foreground size-4 shrink-0" />
                  )}
                  <span className="truncate">{c.name}</span>
                  <span className="text-muted-foreground text-xs font-normal">
                    {c.endpoints.length}
                  </span>
                </button>
                <button
                  className="text-muted-foreground hover:text-primary hidden group-hover:block"
                  onClick={() => set("editing", c)}
                  aria-label={`Settings for ${c.name}`}
                >
                  <Settings2 className="size-3.5" />
                </button>
                <button
                  className="text-muted-foreground hidden group-hover:block hover:text-red-600"
                  onClick={() => set("deleting", c)}
                  aria-label={`Delete ${c.name}`}
                >
                  <Trash2 className="size-3.5" />
                </button>
              </div>
              {isActive && <EndpointList collection={c} />}
            </div>
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
    </div>
  );
};

/** Rename, and variables that act as defaults for this collection's endpoints. */
const CollectionSettingsDialog = ({
  collection,
  onClose,
}: {
  collection: Collection | null;
  onClose: () => void;
}) => {
  const { renameCollection, setCollectionVar } = useWorkspaceStore();
  // Read live values from the store so edits show immediately.
  const live = useWorkspaceStore(
    (s) => s.workspace?.collections.find((c) => c.id === collection?.id) ?? null,
  );
  // The "add variable" row.
  const {
    values: draft,
    set: setDraft,
    reset: clearDraft,
  } = useValues({ initialValue: { key: "", value: "" } });

  return (
    <Dialog open={live !== null} onOpenChange={(open) => !open && onClose()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Collection settings</DialogTitle>
          <DialogDescription>
            Variables here are defaults for this collection&apos;s endpoints. The active
            environment&apos;s variables and secrets override them.
          </DialogDescription>
        </DialogHeader>
        {live && (
          <div className="flex flex-col gap-4 text-sm">
            <label className="flex flex-col gap-1.5">
              <Label>Name</Label>
              <Input
                value={live.name}
                onChange={(e) => renameCollection(live.id, e.target.value)}
              />
            </label>
            <div className="flex flex-col gap-1.5">
              <Label>Variables</Label>
              {Object.entries(live.vars).map(([k, v]) => (
                <div key={k} className="flex items-center gap-1.5">
                  <span className="w-24 shrink-0 truncate font-mono text-xs" title={k}>
                    {k}
                  </span>
                  <Input
                    className="flex-1 font-mono"
                    value={v}
                    onChange={(e) => setCollectionVar(live.id, k, e.target.value)}
                  />
                  <button
                    className="text-muted-foreground hover:text-red-600"
                    onClick={() => setCollectionVar(live.id, k, null)}
                    aria-label={`Remove ${k}`}
                  >
                    <X className="size-4" />
                  </button>
                </div>
              ))}
              <form
                className="flex items-center gap-1.5"
                onSubmit={(e) => {
                  e.preventDefault();
                  if (!draft.key.trim()) return;
                  setCollectionVar(live.id, draft.key.trim(), draft.value);
                  clearDraft();
                }}
              >
                <Input
                  className="w-24 shrink-0"
                  placeholder="base"
                  value={draft.key}
                  onChange={(e) => setDraft("key", e.target.value)}
                />
                <Input
                  className="flex-1 font-mono"
                  placeholder="https://api.example.com"
                  value={draft.value}
                  onChange={(e) => setDraft("value", e.target.value)}
                />
                <button
                  type="submit"
                  className="text-muted-foreground hover:text-primary"
                  aria-label="Add variable"
                >
                  <Plus className="size-4" />
                </button>
              </form>
            </div>
          </div>
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
            This removes the collection and its {count} endpoint{count === 1 ? "" : "s"} from
            kestrel.json.
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
