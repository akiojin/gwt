import React from "react";
import type { CustomAITool } from "../../../../types/api.js";

interface CustomToolListProps {
  tools: CustomAITool[];
  onEdit: (tool: CustomAITool) => void;
  onDelete: (tool: CustomAITool) => void;
}

export function CustomToolList({ tools, onEdit, onDelete }: CustomToolListProps) {
  if (!tools.length) {
    return (
      <div className="page-state page-state--card">
        <h2>カスタムツールが登録されていません</h2>
        <p>「カスタムツールを追加」から最初のツールを登録してください。</p>
      </div>
    );
  }

  return (
    <div className="tool-grid">
      {tools.map((tool) => (
        <article key={tool.id} className="tool-card">
          <header className="tool-card__header">
            <div>
              <p className="tool-card__eyebrow">ID: {tool.id}</p>
              <h3>
                {tool.icon && <span className="tool-card__icon">{tool.icon}</span>}
                {tool.displayName}
              </h3>
            </div>
            <span className="pill">{renderExecutionLabel(tool.executionType)}</span>
          </header>

          <p className="tool-card__command">{tool.command}</p>

          {tool.description && <p className="tool-card__description">{tool.description}</p>}

          <dl className="metadata-grid metadata-grid--compact tool-card__meta">
            <div>
              <dt>normal</dt>
              <dd>{renderArgs(tool.modeArgs?.normal)}</dd>
            </div>
            <div>
              <dt>continue</dt>
              <dd>{renderArgs(tool.modeArgs?.continue)}</dd>
            </div>
            <div>
              <dt>resume</dt>
              <dd>{renderArgs(tool.modeArgs?.resume)}</dd>
            </div>
          </dl>

          <footer className="tool-card__actions">
            <button type="button" className="button button--secondary" onClick={() => onEdit(tool)}>
              編集
            </button>
            <button type="button" className="button button--ghost" onClick={() => onDelete(tool)}>
              削除
            </button>
          </footer>
        </article>
      ))}
    </div>
  );
}

function renderExecutionLabel(type: CustomAITool["executionType"]) {
  switch (type) {
    case "path":
      return "実行ファイル";
    case "bunx":
      return "bunx";
    case "command":
    default:
      return "コマンド";
  }
}

function renderArgs(args?: string[] | null) {
  if (!args || args.length === 0) {
    return <span className="tool-card__muted">未設定</span>;
  }
  return args.join(" ");
}
