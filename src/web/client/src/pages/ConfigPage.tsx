import React, { useEffect, useMemo, useState } from "react";
import { Link } from "react-router-dom";
import type {
  CustomAITool,
  EnvironmentVariable,
} from "../../../../types/api.js";
import { useConfig, useUpdateConfig } from "../hooks/useConfig";
import { CustomToolList } from "../components/CustomToolList";
import {
  CustomToolForm,
  type CustomToolFormValue,
} from "../components/CustomToolForm";
import {
  EnvironmentEditor,
  type EnvEntry,
} from "../components/EnvironmentEditor";
import { PageHeader } from "@/components/common/PageHeader";
import { Button } from "@/components/ui/button";
import { Card, CardHeader, CardContent } from "@/components/ui/card";
import { Alert, AlertDescription } from "@/components/ui/alert";

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
  const [editingTool, setEditingTool] = useState<CustomAITool | undefined>(
    undefined,
  );
  const [banner, setBanner] = useState<BannerState | null>(null);
  const [isCreating, setIsCreating] = useState(false);
  const [envEntries, setEnvEntries] = useState<EnvEntry[]>([]);

  useEffect(() => {
    if (data?.tools) setTools(data.tools);
    if (data) setEnvEntries(entriesFromVariables(data.env));
  }, [data]);

  const sortedTools = useMemo(() => {
    return [...tools].sort((a, b) =>
      a.displayName.localeCompare(b.displayName, "ja"),
    );
  }, [tools]);

  const handleEdit = (tool: CustomAITool) => {
    setEditingTool(tool);
    setIsCreating(false);
  };

  const handleDelete = (tool: CustomAITool) => {
    if (!window.confirm(`${tool.displayName} を削除しますか？`)) return;
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

    persistConfig(
      nextList,
      envEntries,
      `${nextTool.displayName} を保存しました。`,
    );
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
        setBanner({ type: "error", message });
      });
  };

  const handleEnvEntryChange = (
    id: string,
    field: "key" | "value",
    value: string,
  ) => {
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

  const handleEnvAdd = () =>
    setEnvEntries((prev) => [...prev, createEnvEntry()]);
  const handleEnvRemove = (id: string) =>
    setEnvEntries((prev) => prev.filter((e) => e.id !== id));
  const handleEnvSave = () =>
    persistConfig(tools, envEntries, "環境変数を保存しました。", {
      resetToolForm: false,
    });
  const handleCancel = () => {
    setEditingTool(undefined);
    setIsCreating(false);
  };

  const activeFormTool = isCreating ? undefined : editingTool;

  // Loading state
  if (isLoading) {
    return (
      <div className="min-h-screen bg-background">
        <PageHeader
          eyebrow="CONFIGURATION"
          title="カスタムAIツール設定"
          subtitle="読み込み中..."
        />
        <main className="mx-auto max-w-7xl px-6 py-8">
          <div className="flex items-center justify-center py-20">
            <p className="text-muted-foreground">設定を取得しています...</p>
          </div>
        </main>
      </div>
    );
  }

  // Error state
  if (error) {
    return (
      <div className="min-h-screen bg-background">
        <PageHeader eyebrow="CONFIGURATION" title="エラー" />
        <main className="mx-auto max-w-7xl px-6 py-8">
          <Alert variant="destructive">
            <AlertDescription>
              {error instanceof Error ? error.message : "未知のエラーです"}
            </AlertDescription>
          </Alert>
          <div className="mt-4">
            <Button variant="ghost" asChild>
              <Link to="/">← ブランチ一覧に戻る</Link>
            </Button>
          </div>
        </main>
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-background">
      <PageHeader
        eyebrow="CONFIGURATION"
        title="カスタムAIツール設定"
        subtitle="tools.json を編集して、独自のAIツールをCLI / Web UI 両方から利用できます。"
      >
        <div className="mt-4 flex flex-wrap gap-2">
          <Button variant="ghost" size="sm" asChild>
            <Link to="/">← ブランチ一覧</Link>
          </Button>
          <Button variant="secondary" onClick={handleCreate}>
            カスタムツールを追加
          </Button>
        </div>
      </PageHeader>

      {/* Banner */}
      {banner && (
        <div className="mx-auto max-w-7xl px-6 pt-4">
          <Alert
            variant={
              banner.type === "error"
                ? "destructive"
                : banner.type === "success"
                  ? "success"
                  : "info"
            }
          >
            <AlertDescription className="flex items-center justify-between">
              <span>{banner.message}</span>
              <Button variant="ghost" size="sm" onClick={() => setBanner(null)}>
                閉じる
              </Button>
            </AlertDescription>
          </Alert>
        </div>
      )}

      <main className="mx-auto max-w-7xl space-y-6 px-6 py-8">
        {/* Environment Variables */}
        <Card>
          <CardHeader className="pb-3">
            <p className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
              Environment Variables
            </p>
            <h3 className="mt-1 text-lg font-semibold">共有環境変数</h3>
            <p className="mt-2 text-sm text-muted-foreground">
              Web UI で起動する AI
              ツールはここに定義された環境変数を自動的に引き継ぎます。
            </p>
          </CardHeader>
          <CardContent>
            <EnvironmentEditor
              entries={envEntries}
              onEntryChange={handleEnvEntryChange}
              onAddEntry={handleEnvAdd}
              onRemoveEntry={handleEnvRemove}
              onSave={handleEnvSave}
              isSaving={updateConfig.isPending}
            />
          </CardContent>
        </Card>

        {/* Tool List */}
        <Card>
          <CardHeader className="pb-3">
            <p className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
              Registered Tools
            </p>
            <h3 className="mt-1 text-lg font-semibold">登録済みツール</h3>
            <p className="mt-2 text-sm text-muted-foreground">
              CLI と Web UI は同じ設定を参照します。更新すると
              ~/.claude-worktree/tools.json に保存されます。
            </p>
          </CardHeader>
          <CardContent>
            <CustomToolList
              tools={sortedTools}
              onEdit={handleEdit}
              onDelete={handleDelete}
            />
          </CardContent>
        </Card>

        {/* Tool Form */}
        {(isCreating || editingTool) && (
          <Card>
            <CardContent className="pt-6">
              <CustomToolForm
                {...(activeFormTool ? { initialValue: activeFormTool } : {})}
                onSubmit={handleFormSubmit}
                onCancel={handleCancel}
                isSaving={updateConfig.isPending}
              />
            </CardContent>
          </Card>
        )}
      </main>
    </div>
  );
}

// Helpers
function sanitizeEnvKey(value: string): string {
  return value
    .toUpperCase()
    .replace(/[^A-Z0-9_]/g, "")
    .slice(0, ENV_KEY_MAX);
}

function createEnvEntry(initial?: Partial<EnvEntry>): EnvEntry {
  return {
    id: `${initial?.key ?? "env"}-${Math.random().toString(36).slice(2, 8)}${Date.now().toString(36)}`,
    key: initial?.key ?? "",
    value: initial?.value ?? "",
  };
}

function entriesFromVariables(
  variables?: EnvironmentVariable[] | null,
): EnvEntry[] {
  if (!variables?.length) return [];
  return [...variables]
    .sort((a, b) => a.key.localeCompare(b.key, "en"))
    .map((v) => createEnvEntry({ key: v.key, value: v.value }));
}

function buildEnvVariables(entries: EnvEntry[]): EnvironmentVariable[] {
  const result: EnvironmentVariable[] = [];
  const seen = new Set<string>();
  const timestamp = new Date().toISOString();

  for (const entry of entries) {
    const key = entry.key.trim();
    const value = entry.value;
    if (!key && !value) continue;

    if (!key) throw new Error("環境変数のキーを入力してください。");
    if (!ENV_KEY_REGEX.test(key))
      throw new Error(
        "環境変数キーは英大文字・数字・アンダースコアのみ使用できます。",
      );
    if (key.length > ENV_KEY_MAX)
      throw new Error(`環境変数キーは最大${ENV_KEY_MAX}文字です。(${key})`);
    if (!value) throw new Error(`${key} の値を入力してください。`);
    if (value.length > ENV_VALUE_MAX)
      throw new Error(`${key} の値は最大${ENV_VALUE_MAX}文字です。`);
    if (seen.has(key))
      throw new Error(`環境変数キー "${key}" が重複しています。`);
    seen.add(key);

    result.push({ key, value, lastUpdated: timestamp });
  }

  return result;
}
