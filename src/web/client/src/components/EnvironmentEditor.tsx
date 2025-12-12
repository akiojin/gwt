import React from "react";

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
    <div className="env-editor">
      <p className="env-editor__description">
        Claude Code / Codex CLI などが参照する共有環境変数を管理します。例: ANTHROPIC_API_KEY, OPENAI_API_KEY, GITHUB_TOKEN
      </p>

      <div className="env-editor__rows">
        {!hasEntries && (
          <p className="env-editor__empty">環境変数はまだ設定されていません。</p>
        )}

        {entries.map((entry) => (
          <div className="env-editor__row" key={entry.id}>
            <label className="form-field">
              <span>キー</span>
              <input
                type="text"
                value={entry.key}
                onChange={(event) => onEntryChange(entry.id, "key", event.target.value)}
                placeholder="ANTHROPIC_API_KEY"
                maxLength={100}
                disabled={isSaving}
              />
            </label>

            <label className="form-field form-field--stacked">
              <span>値</span>
              <input
                type="text"
                value={entry.value}
                onChange={(event) => onEntryChange(entry.id, "value", event.target.value)}
                placeholder="sk-..."
                maxLength={500}
                disabled={isSaving}
              />
            </label>

            <button
              type="button"
              className="button button--ghost env-editor__remove"
              onClick={() => onRemoveEntry(entry.id)}
              disabled={isSaving}
            >
              削除
            </button>
          </div>
        ))}
      </div>

      <div className="env-editor__actions">
        <button
          type="button"
          className="button button--secondary"
          onClick={onAddEntry}
          disabled={isSaving}
        >
          環境変数を追加
        </button>

        <button
          type="button"
          className="button button--primary"
          onClick={onSave}
          disabled={isSaving}
        >
          {isSaving ? "保存中..." : "保存"}
        </button>
      </div>
    </div>
  );
}

