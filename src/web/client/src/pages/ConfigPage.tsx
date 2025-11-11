import React, { useEffect, useMemo, useState } from "react";
import { Link } from "react-router-dom";
import type { CustomAITool } from "../../../../types/api.js";
import { useConfig, useUpdateConfig } from "../hooks/useConfig";
import { CustomToolList } from "../components/CustomToolList";
import { CustomToolForm, type CustomToolFormValue } from "../components/CustomToolForm";

interface BannerState {
  type: "success" | "error" | "info";
  message: string;
}

export function ConfigPage() {
  const { data, isLoading, error } = useConfig();
  const updateConfig = useUpdateConfig();
  const [tools, setTools] = useState<CustomAITool[]>([]);
  const [editingTool, setEditingTool] = useState<CustomAITool | undefined>(undefined);
  const [banner, setBanner] = useState<BannerState | null>(null);
  const [isCreating, setIsCreating] = useState(false);

  useEffect(() => {
    if (data?.tools) {
      setTools(data.tools);
    }
  }, [data?.tools]);

  const sortedTools = useMemo(() => {
    return [...tools].sort((a, b) => a.displayName.localeCompare(b.displayName, "ja"));
  }, [tools]);

  const handleEdit = (tool: CustomAITool) => {
    setEditingTool(tool);
    setIsCreating(false);
  };

  const handleDelete = (tool: CustomAITool) => {
    if (!window.confirm(`${tool.displayName} を削除しますか？`)) {
      return;
    }

    const next = tools.filter((t) => t.id !== tool.id);
    persistTools(next, `${tool.displayName} を削除しました。`);
  };

  const handleCreate = () => {
    setEditingTool(undefined);
    setIsCreating(true);
  };

  const handleFormSubmit = (value: CustomToolFormValue) => {
    const now = new Date().toISOString();
    const existing = tools.find((tool) => tool.id === value.id);
    const nextTool: CustomAITool = {
      id: value.id,
      displayName: value.displayName,
      icon: value.icon ?? null,
      description: value.description ?? null,
      executionType: value.executionType,
      command: value.command,
      defaultArgs: value.defaultArgs ?? null,
      modeArgs: {
        normal: value.modeArgs.normal ?? [],
        continue: value.modeArgs.continue ?? [],
        resume: value.modeArgs.resume ?? [],
      },
      permissionSkipArgs: value.permissionSkipArgs ?? null,
      env: value.env ?? null,
      createdAt: existing?.createdAt ?? now,
      updatedAt: now,
    };

    const nextList = existing
      ? tools.map((tool) => (tool.id === nextTool.id ? nextTool : tool))
      : [...tools, nextTool];

    persistTools(nextList, `${nextTool.displayName} を保存しました。`);
  };

  const persistTools = (nextTools: CustomAITool[], successMessage: string) => {
    updateConfig
      .mutateAsync(nextTools)
      .then((response) => {
        setTools(response.tools);
        setBanner({ type: "success", message: successMessage });
        setEditingTool(undefined);
        setIsCreating(false);
      })
      .catch((err) => {
        const message = err instanceof Error ? err.message : String(err);
        setBanner({ type: "error", message: message });
      });
  };

  const handleCancel = () => {
    setEditingTool(undefined);
    setIsCreating(false);
  };

  const activeFormTool = isCreating ? undefined : editingTool;

  if (isLoading) {
    return (
      <div className="app-shell">
        <div className="page-state page-state--centered">
          <h1>読み込み中</h1>
          <p>設定を取得しています...</p>
        </div>
      </div>
    );
  }

  if (error) {
    return (
      <div className="app-shell">
        <div className="page-state page-state--centered">
          <h1>設定の取得に失敗しました</h1>
          <p>{error instanceof Error ? error.message : "未知のエラーです"}</p>
          <Link to="/" className="button button--ghost">
            ブランチ一覧に戻る
          </Link>
        </div>
      </div>
    );
  }

  return (
    <div className="app-shell">
      <header className="page-hero page-hero--compact">
        <Link to="/" className="link-back">
          ← ブランチ一覧に戻る
        </Link>
        <p className="page-hero__eyebrow">CONFIGURATION</p>
        <h1>カスタムAIツール設定</h1>
        <p className="page-hero__subtitle">
          tools.json を編集して、独自のAIツールをCLI / Web UI 両方から利用できます。
        </p>
        <div className="page-hero__actions">
          <button type="button" className="button button--secondary" onClick={handleCreate}>
            カスタムツールを追加
          </button>
        </div>
      </header>

      <main className="page-content page-content--wide">
        {banner && <InlineBanner banner={banner} onClose={() => setBanner(null)} />}

        <section className="section-card">
          <header className="section-card__header">
            <div>
              <h2>登録済みツール</h2>
              <p className="section-card__body">
                CLI と Web UI は同じ設定を参照します。ここで更新すると ~/.claude-worktree/tools.json に保存されます。
              </p>
            </div>
          </header>

          <CustomToolList tools={sortedTools} onEdit={handleEdit} onDelete={handleDelete} />
        </section>

        {(isCreating || editingTool) && (
          <section className="section-card">
            <CustomToolForm
              {...(activeFormTool ? { initialValue: activeFormTool } : {})}
              onSubmit={handleFormSubmit}
              onCancel={handleCancel}
              isSaving={updateConfig.isPending}
            />
          </section>
        )}
      </main>
    </div>
  );
}

function InlineBanner({ banner, onClose }: { banner: BannerState; onClose: () => void }) {
  return (
    <div className={`inline-banner inline-banner--${banner.type}`}>
      <div className="inline-banner__content">
        <span>{banner.message}</span>
        <button type="button" className="button button--ghost" onClick={onClose}>
          閉じる
        </button>
      </div>
    </div>
  );
}
