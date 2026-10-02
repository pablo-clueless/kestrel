"use client";

import { Minus, Plus, Trash2 } from "lucide-react";
import { z } from "zod";

import { useWorkspaceStore } from "@/stores/workspace-store";
import { AddPairForm } from "@/components/shared/add-pair-form";
import { Form } from "@/components/form";
import { Input, Label } from "./fields";

const envSchema = z.object({ name: z.string() });

const addButton = "text-muted-foreground hover:text-green-500 shrink-0";

/** Variables and secrets for the active environment. Secret values are write-only. */
export const EnvironmentEditor = () => {
  const { workspace, secretKeys, addEnvironment, removeEnvironment, setVar, setSecret } =
    useWorkspaceStore();
  const active = workspace?.environments.find((e) => e.name === workspace.activeEnvironment);

  return (
    <div className="flex flex-col gap-4 text-sm">
      <Form
        schema={envSchema}
        defaultValues={{ name: "" }}
        fields={{
          name: {
            type: "text",
            label: "New environment",
            hideLabel: true,
            placeholder: "New environment",
            autoComplete: "off",
            className: "flex-1",
          },
        }}
        onSubmit={({ name }, form) => {
          if (!name) return;
          addEnvironment(name);
          form.reset();
        }}
      >
        {({ field }) => (
          <div className="flex items-center gap-1.5">
            {field("name")}
            <button type="submit" className={addButton} aria-label="Add environment">
              <Plus className="size-4" />
            </button>
          </div>
        )}
      </Form>
      {!active ? (
        <p className="text-muted-foreground">Pick an environment in the header, or create one.</p>
      ) : (
        <div className="space-y-2">
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
              <div key={key} className="flex items-center gap-3">
                <span className="w-16 shrink-0 truncate font-mono text-xs" title={key}>
                  {key}
                </span>
                <Input
                  className="flex-1 font-mono"
                  value={value}
                  onChange={(e) => setVar(active.name, key, e.target.value)}
                />
                <button
                  className="text-muted-foreground hover:text-red-500"
                  onClick={() => setVar(active.name, key, null)}
                  aria-label={`Remove ${key}`}
                >
                  <Minus className="size-4" />
                </button>
              </div>
            ))}
            <AddPairForm noun="Variable" onAdd={(k, v) => setVar(active.name, k, v)} />
          </section>
          <section className="flex flex-col gap-1.5">
            <Label>Secrets</Label>
            {(secretKeys[active.name] ?? []).map((key) => (
              <div key={key} className="flex items-center gap-3">
                <span className="w-16 shrink-0 truncate font-mono text-xs" title={key}>
                  {key}
                </span>
                <span className="text-muted-foreground flex-1">••••••</span>
                <button
                  className="text-muted-foreground hover:text-red-500"
                  onClick={() => void setSecret(active.name, key, null)}
                  aria-label={`Remove ${key}`}
                >
                  <Minus className="size-4" />
                </button>
              </div>
            ))}
            <AddPairForm
              noun="Secret"
              secret
              onAdd={(k, v) => void setSecret(active.name, k, v)}
              valuePlaceholder="value (write-only)"
            />
          </section>
        </div>
      )}
    </div>
  );
};
