import React, { useEffect, useMemo, useState } from "react";
import { Link } from "react-router-dom";
import type {
  CustomAITool,
  EnvironmentVariable,
} from "../../../../types/api.js";
import { useConfig, useUpdateConfig } from "../hooks/useConfig";
import { CustomToolList } from "../components/CustomToolList";
import { CustomToolForm, type CustomToolFormValue } from "../components/CustomToolForm";
import {
  EnvironmentEditor,
  type EnvEntry,
} from "../components/EnvironmentEditor";

interface BannerState {
  type: "success" | "error" | "info";
  message: string;
}

const ENV_KEY_REGEX = /^[A-Z0-9_]+$/;
const ENV_KEY_MAX = 100;
const ENV_VALUE_MAX = 500;

export function ConfigPage() {
  const { data, isLoading, error } = useConfig();
  const updateConfig = useUpdateConfig();
  const [tools, setTools] = useState<CustomAITool[]>([]);
  const [editingTool, setEditingTool] = useState<CustomAITool | undefined>(undefined);
  const [banner, setBanner] = useState<BannerState | null>(null);
  const [isCreating, setIsCreating] = useState(false);
  const [envEntries, setEnvEntries] = useState<EnvEntry[]>([]);

  useEffect(() => {
    if (data?.tools) {
      setTools(data.tools);
    }
    if (data) {
      setEnvEntries(entriesFromVariables(data.env));
    }
  }, [data]);

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
    persistConfig(next, envEntries, `${tool.displayName} を削除しました。`);
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

    persistConfig(nextList, envEntries, `${nextTool.displayName} を保存しました。`);
  };

  const persistConfig = (
    nextTools: CustomAITool[],
    nextEnvEntries: EnvEntry[],
    successMessage: string,
    options?: { resetToolForm?: boolean },
  ) => {
    let envPayload: EnvironmentVariable[];
    try {
      envPayload = buildEnvVariables(nextEnvEntries);
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setBanner({ type: "error", message });
      return;
    }

    const nextVersion = data?.version ?? "1.0.0";
    updateConfig
      .mutateAsync({ version: nextVersion, tools: nextTools, env: envPayload })
      .then((response) => {
        setTools(response.tools);
        setEnvEntries(entriesFromVariables(response.env));
        setBanner({ type: "success", message: successMessage });
        if (options?.resetToolForm ?? true) {
          setEditingTool(undefined);
          setIsCreating(false);
        }
      })
      .catch((err) => {
        const message = err instanceof Error ? err.message : String(err);
        setBanner({ type: "error", message: message });
      });
  };

  const handleEnvEntryChange = (id: string, field: "key" | "value", value: string) => {
    setEnvEntries((prev) =>
      prev.map((entry) =>
        entry.id === id
          ? {
              ...entry,
              [field]: field === "key" ? sanitizeEnvKey(value) : value,
            }
          : entry,
      ),
    );
  };

  const handleEnvAdd = () => {
    setEnvEntries((prev) => [...prev, createEnvEntry()]);
  };

  const handleEnvRemove = (id: string) => {
    setEnvEntries((prev) => prev.filter((entry) => entry.id !== id));
  };

  const handleEnvSave = () => {
    persistConfig(tools, envEntries, "環境変数を保存しました。", { resetToolForm: false });
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
              <h2>共有環境変数</h2>
              <p className="section-card__body">
                Web UI で起動する AI ツールはここに定義された環境変数を自動的に引き継ぎます。
                OS に設定済みの ANTHROPIC_API_KEY や OPENAI_API_KEY は初回起動時に自動で取り込まれます。
              </p>
            </div>
          </header>

          <EnvironmentEditor
            entries={envEntries}
            onEntryChange={handleEnvEntryChange}
            onAddEntry={handleEnvAdd}
            onRemoveEntry={handleEnvRemove}
            onSave={handleEnvSave}
            isSaving={updateConfig.isPending}
          />
        </section>

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

function sanitizeEnvKey(value: string): string {
  return value.toUpperCase().replace(/[^A-Z0-9_]/g, "").slice(0, ENV_KEY_MAX);
}

function createEnvEntry(initial?: Partial<EnvEntry>): EnvEntry {
  return {
    id: createEnvEntryId(initial?.key),
    key: initial?.key ?? "",
    value: initial?.value ?? "",
  };
}

function createEnvEntryId(seed?: string): string {
  const random = Math.random().toString(36).slice(2, 8);
  const timestamp = Date.now().toString(36);
  return `${seed ?? "env"}-${random}${timestamp}`;
}

function entriesFromVariables(
  variables?: EnvironmentVariable[] | null,
): EnvEntry[] {
  if (!variables || variables.length === 0) {
    return [];
  }

  return [...variables]
    .sort((a, b) => a.key.localeCompare(b.key, "en"))
    .map((variable) =>
      createEnvEntry({
        key: variable.key,
        value: variable.value,
      }),
    );
}

function buildEnvVariables(entries: EnvEntry[]): EnvironmentVariable[] {
  const result: EnvironmentVariable[] = [];
  const seen = new Set<string>();
  const timestamp = new Date().toISOString();

  for (const entry of entries) {
    const key = entry.key.trim();
    const value = entry.value;
    const isBlank = key.length === 0 && value.length === 0;
    if (isBlank) {
      continue;
    }

    if (!key) {
      throw new Error("環境変数のキーを入力してください。");
    }
    if (!ENV_KEY_REGEX.test(key)) {
      throw new Error(
        "環境変数キーは英大文字・数字・アンダースコアのみ使用できます。",
      );
    }
    if (key.length > ENV_KEY_MAX) {
      throw new Error(`環境変数キーは最大${ENV_KEY_MAX}文字です。(${key})`);
    }
    if (!value) {
      throw new Error(`${key} の値を入力してください。`);
    }
    if (value.length > ENV_VALUE_MAX) {
      throw new Error(`${key} の値は最大${ENV_VALUE_MAX}文字です。`);
    }
    if (seen.has(key)) {
      throw new Error(`環境変数キー "${key}" が重複しています。`);
    }
    seen.add(key);

    result.push({
      key,
      value,
      lastUpdated: timestamp,
    });
  }

  return result;
}
