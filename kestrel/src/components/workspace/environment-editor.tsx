"use client";

import { Plus, Trash2, X } from "lucide-react";
import { useState } from "react";

import { useWorkspaceStore } from "@/stores/workspace-store";
import { Input, Label } from "./fields";

/** Variables and secrets for the active environment. Secret values are write-only. */
export const EnvironmentEditor = () => {
  const { workspace, secretKeys, addEnvironment, removeEnvironment, setVar, setSecret } =
    useWorkspaceStore();
  const [newEnv, setNewEnv] = useState("");
  const active = workspace?.environments.find((e) => e.name === workspace.activeEnvironment);

  const create = () => {
    const name = newEnv.trim();
    if (!name) return;
    addEnvironment(name);
    setNewEnv("");
  };

  return (
    <div className="flex flex-col gap-4 text-sm">
      <form
        className="flex gap-1.5"
        onSubmit={(e) => {
          e.preventDefault();
          create();
        }}
      >
        <Input
          className="flex-1"
          placeholder="New environment"
          value={newEnv}
          onChange={(e) => setNewEnv(e.target.value)}
        />
        <button
          type="submit"
          className="text-muted-foreground hover:text-primary"
          aria-label="Add environment"
        >
          <Plus className="size-4" />
        </button>
      </form>
      {!active ? (
        <p className="text-muted-foreground">Pick an environment in the header, or create one.</p>
      ) : (
        <>
          <div className="flex items-center justify-between">
            <span className="font-medium">{active.name}</span>
            <button
              className="text-muted-foreground hover:text-destructive"
              onClick={() => removeEnvironment(active.name)}
              aria-label={`Delete ${active.name}`}
            >
              <Trash2 className="size-3.5" />
            </button>
          </div>

          <section className="flex flex-col gap-1.5">
            <Label>Variables</Label>
            {Object.entries(active.vars).map(([key, value]) => (
              <div key={key} className="flex items-center gap-1.5">
                <span className="w-16 shrink-0 truncate font-mono text-xs" title={key}>
                  {key}
                </span>
                <Input
                  className="flex-1 font-mono"
                  value={value}
                  onChange={(e) => setVar(active.name, key, e.target.value)}
                />
                <button
                  className="text-muted-foreground hover:text-destructive"
                  onClick={() => setVar(active.name, key, null)}
                  aria-label={`Remove ${key}`}
                >
                  <X className="size-4" />
                </button>
              </div>
            ))}
            <NewPair onAdd={(k, v) => setVar(active.name, k, v)} valuePlaceholder="value" />
          </section>

          <section className="flex flex-col gap-1.5">
            <Label>Secrets</Label>
            {(secretKeys[active.name] ?? []).map((key) => (
              <div key={key} className="flex items-center gap-1.5">
                <span className="w-16 shrink-0 truncate font-mono text-xs" title={key}>
                  {key}
                </span>
                <span className="text-muted-foreground flex-1">••••••</span>
                <button
                  className="text-muted-foreground hover:text-destructive"
                  onClick={() => void setSecret(active.name, key, null)}
                  aria-label={`Remove ${key}`}
                >
                  <X className="size-4" />
                </button>
              </div>
            ))}
            <NewPair
              secret
              onAdd={(k, v) => void setSecret(active.name, k, v)}
              valuePlaceholder="value (write-only)"
            />
          </section>
        </>
      )}
    </div>
  );
};

const NewPair = ({
  onAdd,
  valuePlaceholder,
  secret,
}: {
  onAdd: (key: string, value: string) => void;
  valuePlaceholder: string;
  secret?: boolean;
}) => {
  const [key, setKey] = useState("");
  const [value, setValue] = useState("");
  return (
    <form
      className="flex items-center gap-1.5"
      onSubmit={(e) => {
        e.preventDefault();
        if (!key.trim()) return;
        onAdd(key.trim(), value);
        setKey("");
        setValue("");
      }}
    >
      <Input
        className="w-16 shrink-0"
        placeholder="name"
        value={key}
        onChange={(e) => setKey(e.target.value)}
      />
      <Input
        className="flex-1 font-mono"
        type={secret ? "password" : "text"}
        autoComplete="off"
        placeholder={valuePlaceholder}
        value={value}
        onChange={(e) => setValue(e.target.value)}
      />
      <button type="submit" className="text-muted-foreground hover:text-primary" aria-label="Add">
        <Plus className="size-4" />
      </button>
    </form>
  );
};
