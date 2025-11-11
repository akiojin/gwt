import React from "react";

export interface EnvRow {
  id: string;
  key: string;
  value: string;
  importedFromOs?: boolean;
  lastUpdated?: string | null;
}

interface EnvEditorProps {
  title: string;
  rows: EnvRow[];
  onChange: (rows: EnvRow[]) => void;
  description?: string;
  allowAdd?: boolean;
  emptyLabel?: string;
}

const KEY_PATTERN = /^[A-Z0-9_]+$/;

export function createEnvRow(variable?: Partial<EnvRow>): EnvRow {
  const row: EnvRow = {
    id: variable?.id ?? `env-${Date.now()}-${Math.random().toString(36).slice(2)}`,
    key: variable?.key ?? "",
    value: variable?.value ?? "",
  };

  if (typeof variable?.importedFromOs === "boolean") {
    row.importedFromOs = variable.importedFromOs;
  }
  if (variable?.lastUpdated) {
    row.lastUpdated = variable.lastUpdated;
  }

  return row;
}

function isInvalidKey(row: EnvRow): boolean {
  if (!row.key) return true;
  return !KEY_PATTERN.test(row.key);
}

export function EnvEditor({
  title,
  rows,
  onChange,
  description,
  allowAdd = true,
  emptyLabel = "環境変数はまだありません",
}: EnvEditorProps) {
  const handleFieldChange = (id: string, field: "key" | "value", value: string) => {
    onChange(
      rows.map((row) =>
        row.id === id
          ? {
              ...row,
              [field]: field === "key" ? value.toUpperCase().replace(/[^A-Z0-9_]/g, "_") : value,
            }
          : row,
      ),
    );
  };

  const handleRemove = (id: string) => {
    onChange(rows.filter((row) => row.id !== id));
  };

  const handleAdd = () => {
    onChange([...rows, createEnvRow()]);
  };

  return (
    <div className="env-editor">
      <header className="env-editor__header">
        <div>
          <h3>{title}</h3>
          {description && <p className="env-editor__description">{description}</p>}
        </div>
        {allowAdd && (
          <button type="button" className="button button--secondary" onClick={handleAdd}>
            変数を追加
          </button>
        )}
      </header>

      {rows.length === 0 ? (
        <p className="env-editor__empty">{emptyLabel}</p>
      ) : (
        <table className="env-editor__table">
          <thead>
            <tr>
              <th>キー</th>
              <th>値</th>
              <th style={{ width: "140px" }}>操作</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((row) => {
              const keyInvalid = isInvalidKey(row);
              return (
                <tr key={row.id} className={keyInvalid ? "env-editor__row--invalid" : undefined}>
                  <td>
                    <input
                      type="text"
                      value={row.key}
                      onChange={(event) => handleFieldChange(row.id, "key", event.target.value)}
                      placeholder="EXAMPLE_KEY"
                    />
                    {row.importedFromOs && (
                      <span className="pill pill--info" style={{ marginLeft: "0.5rem" }}>
                        OSから取り込み
                      </span>
                    )}
                    {row.lastUpdated && (
                      <span className="env-editor__meta">更新: {new Date(row.lastUpdated).toLocaleString()}</span>
                    )}
                    {keyInvalid && <p className="env-editor__error">A-Z,0-9,_ のみ使用できます</p>}
                  </td>
                  <td>
                    <input
                      type="text"
                      value={row.value}
                      onChange={(event) => handleFieldChange(row.id, "value", event.target.value)}
                      placeholder="値"
                    />
                  </td>
                  <td>
                    <button
                      type="button"
                      className="button button--ghost"
                      onClick={() => handleRemove(row.id)}
                    >
                      削除
                    </button>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
    </div>
  );
}
