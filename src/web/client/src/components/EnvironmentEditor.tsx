import React from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

export interface EnvEntry {
  id: string;
  key: string;
  value: string;
}

interface EnvironmentEditorProps {
  entries: EnvEntry[];
  onEntryChange: (id: string, field: "key" | "value", value: string) => void;
  onAddEntry: () => void;
  onRemoveEntry: (id: string) => void;
  onSave: () => void;
  isSaving?: boolean;
}

export function EnvironmentEditor({
  entries,
  onEntryChange,
  onAddEntry,
  onRemoveEntry,
  onSave,
  isSaving,
}: EnvironmentEditorProps) {
  const hasEntries = entries.length > 0;

  return (
    <div className="space-y-4">
      <p className="text-sm text-muted-foreground">
        Claude Code / Codex CLI などが参照する共有環境変数を管理します。例:
        ANTHROPIC_API_KEY, OPENAI_API_KEY, GITHUB_TOKEN
      </p>

      <div className="space-y-3">
        {!hasEntries && (
          <p className="py-4 text-center text-sm text-muted-foreground">
            環境変数はまだ設定されていません。
          </p>
        )}

        {entries.map((entry) => (
          <div key={entry.id} className="flex items-end gap-3">
            <div className="flex-1 space-y-1">
              <label className="text-sm font-medium">キー</label>
              <Input
                type="text"
                value={entry.key}
                onChange={(event) =>
                  onEntryChange(entry.id, "key", event.target.value)
                }
                placeholder="ANTHROPIC_API_KEY"
                maxLength={100}
                disabled={isSaving}
              />
            </div>

            <div className="flex-1 space-y-1">
              <label className="text-sm font-medium">値</label>
              <Input
                type="text"
                value={entry.value}
                onChange={(event) =>
                  onEntryChange(entry.id, "value", event.target.value)
                }
                placeholder="sk-..."
                maxLength={500}
                disabled={isSaving}
              />
            </div>

            <Button
              variant="ghost"
              size="sm"
              onClick={() => onRemoveEntry(entry.id)}
              disabled={isSaving}
            >
              削除
            </Button>
          </div>
        ))}
      </div>

      <div className="flex gap-2 pt-2">
        <Button variant="secondary" onClick={onAddEntry} disabled={isSaving}>
          環境変数を追加
        </Button>

        <Button onClick={onSave} disabled={isSaving}>
          {isSaving ? "保存中..." : "保存"}
        </Button>
      </div>
    </div>
  );
}
