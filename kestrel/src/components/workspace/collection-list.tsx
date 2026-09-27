"use client";

import { ChevronDown, ChevronRight, FileUp, Plus, Settings2, Trash2, X } from "lucide-react";
import { useEffect, useState } from "react";

import { activeCollectionOf, useWorkspaceStore } from "@/stores/workspace-store";
import type { Collection } from "@/types/engine/Collection";
import { Button } from "@/components/ui/button";
import { EndpointList } from "./endpoint-list";
import { ImportDialog } from "./import-dialog";
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

/** Sidebar: collections of endpoints. The active one is expanded; clicking another switches to it.
 * Also loads the workspace. */
export const CollectionList = () => {
  const { workspace, loadError, load, setActiveCollection, addCollection } = useWorkspaceStore();
  const [adding, setAdding] = useState(false);
  const [newName, setNewName] = useState("");
  const [editing, setEditing] = useState<Collection | null>(null);
  const [deleting, setDeleting] = useState<Collection | null>(null);
  const [importing, setImporting] = useState(false);
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
    setNewName("");
    setAdding(false);
  };

  return (
    <div className="flex h-full flex-col gap-3">
      <div className="flex items-center justify-between">
        <span className="text-muted-foreground text-xs uppercase">Collections</span>
        <div className="flex items-center gap-2">
          <button
            className="text-muted-foreground hover:text-primary"
            onClick={() => setImporting(true)}
            disabled={!workspace}
            aria-label="Import API spec"
            title="Import OpenAPI / Swagger"
          >
            <FileUp className="size-4" />
          </button>
          <button
            className="text-muted-foreground hover:text-primary"
            onClick={() => setAdding(true)}
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
            onChange={(e) => setNewName(e.target.value)}
            onBlur={create}
            onKeyDown={(e) => e.key === "Escape" && setAdding(false)}
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
                  onClick={() => setEditing(c)}
                  aria-label={`Settings for ${c.name}`}
                >
                  <Settings2 className="size-3.5" />
                </button>
                <button
                  className="text-muted-foreground hidden group-hover:block hover:text-red-600"
                  onClick={() => setDeleting(c)}
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

      <CollectionSettingsDialog collection={editing} onClose={() => setEditing(null)} />
      <DeleteCollectionDialog collection={deleting} onClose={() => setDeleting(null)} />
      <ImportDialog open={importing} onClose={() => setImporting(false)} />
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
  const [key, setKey] = useState("");
  const [value, setValue] = useState("");

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
                  if (!key.trim()) return;
                  setCollectionVar(live.id, key.trim(), value);
                  setKey("");
                  setValue("");
                }}
              >
                <Input
                  className="w-24 shrink-0"
                  placeholder="base"
                  value={key}
                  onChange={(e) => setKey(e.target.value)}
                />
                <Input
                  className="flex-1 font-mono"
                  placeholder="https://api.example.com"
                  value={value}
                  onChange={(e) => setValue(e.target.value)}
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
