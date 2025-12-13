import React, { useEffect, useMemo, useState } from "react";
import { Link } from "react-router-dom";
import type {
  ConfigPayload,
  CustomAITool,
  EnvironmentVariable,
} from "../../../../types/api.js";
import { useConfig, useUpdateConfig } from "../hooks/useConfig";
import { EnvEditor, createEnvRow, type EnvRow } from "../components/EnvEditor";
import { PageHeader } from "@/components/common/PageHeader";
import { Button } from "@/components/ui/button";
import { Card, CardHeader, CardContent } from "@/components/ui/card";
import { Alert, AlertDescription } from "@/components/ui/alert";

type ToolEnvState = Record<string, EnvRow[]>;

function rowsFromVariables(variables?: EnvironmentVariable[] | null): EnvRow[] {
  if (!variables) return [];
  return variables.map((variable) => {
    const partial: Partial<EnvRow> = {
      key: variable.key,
      value: variable.value,
    };
    if (typeof variable.importedFromOs === "boolean")
      partial.importedFromOs = variable.importedFromOs;
    if (variable.lastUpdated) partial.lastUpdated = variable.lastUpdated;
    return createEnvRow(partial);
  });
}

function serializeRows(rows: EnvRow[]): EnvironmentVariable[] {
  return rows
    .filter((row) => row.key.trim().length > 0)
    .map((row) => ({ key: row.key.trim().toUpperCase(), value: row.value }));
}

function buildPayload(
  base: ConfigPayload | undefined,
  sharedEnv: EnvRow[],
  toolState: ToolEnvState,
): ConfigPayload {
  const tools: CustomAITool[] = (base?.tools ?? []).map((tool) => ({
    ...tool,
    env: serializeRows(toolState[tool.id] ?? []),
  }));

  return {
    version: base?.version ?? "1.0.0",
    env: serializeRows(sharedEnv),
    tools,
  };
}

export function ConfigManagementPage() {
  const { data, isLoading, error } = useConfig();
  const updateConfig = useUpdateConfig();
  const [sharedEnv, setSharedEnv] = useState<EnvRow[]>([]);
  const [toolEnv, setToolEnv] = useState<ToolEnvState>({});
  const [banner, setBanner] = useState<{
    type: "success" | "error";
    message: string;
  } | null>(null);

  useEffect(() => {
    if (!data) return;
    setSharedEnv(rowsFromVariables(data.env));
    const toolState: ToolEnvState = {};
    data.tools?.forEach((tool) => {
      toolState[tool.id] = rowsFromVariables(tool.env);
    });
    setToolEnv(toolState);
  }, [data]);

  const serializedOriginalShared = useMemo(
    () => JSON.stringify(data?.env ?? []),
    [data?.env],
  );
  const serializedCurrentShared = useMemo(
    () => JSON.stringify(serializeRows(sharedEnv)),
    [sharedEnv],
  );

  const hasInvalidRows = useMemo(() => {
    const keyInvalid = sharedEnv.some(
      (row) => !row.key || /[^A-Z0-9_]/.test(row.key),
    );
    const valueInvalid = sharedEnv.some(
      (row) => row.key && row.value.trim().length === 0,
    );
    const toolInvalid = Object.values(toolEnv).some((rows) =>
      rows.some(
        (row) =>
          !row.key ||
          /[^A-Z0-9_]/.test(row.key) ||
          row.value.trim().length === 0,
      ),
    );
    return keyInvalid || valueInvalid || toolInvalid;
  }, [sharedEnv, toolEnv]);

  const hasChanges = useMemo(() => {
    if (serializedOriginalShared !== serializedCurrentShared) return true;
    if (!data) return false;
    const currentTool =
      data.tools?.map((tool) => serializeRows(toolEnv[tool.id] ?? [])) ?? [];
    const originalTool = data.tools?.map((tool) => tool.env ?? []) ?? [];
    return JSON.stringify(currentTool) !== JSON.stringify(originalTool);
  }, [data, serializedOriginalShared, serializedCurrentShared, toolEnv]);

  const handleSave = async () => {
    if (!data) return;
    try {
      const payload = buildPayload(data, sharedEnv, toolEnv);
      await updateConfig.mutateAsync(payload);
      setBanner({ type: "success", message: "設定を保存しました" });
    } catch (err) {
      setBanner({
        type: "error",
        message: err instanceof Error ? err.message : "保存に失敗しました",
      });
    }
  };

  // Loading state
  if (isLoading) {
    return (
      <div className="min-h-screen bg-background">
        <PageHeader
          eyebrow="CONFIG"
          title="環境変数の管理"
          subtitle="読み込み中..."
        />
        <main className="mx-auto max-w-7xl px-6 py-8">
          <div className="flex items-center justify-center py-20">
            <p className="text-muted-foreground">設定を読み込んでいます...</p>
          </div>
        </main>
      </div>
    );
  }

  // Error state
  if (error) {
    return (
      <div className="min-h-screen bg-background">
        <PageHeader eyebrow="CONFIG" title="エラー" />
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
        eyebrow="CONFIG"
        title="環境変数の管理"
        subtitle="共通環境変数とツールごとの上書きをブラウザから編集できます。"
      >
        <div className="mt-4 flex flex-wrap gap-2">
          <Button variant="ghost" size="sm" asChild>
            <Link to="/">← ブランチ一覧へ</Link>
          </Button>
          <Button
            onClick={handleSave}
            disabled={updateConfig.isPending || hasInvalidRows || !hasChanges}
          >
            {updateConfig.isPending ? "保存中..." : "保存"}
          </Button>
        </div>
      </PageHeader>

      {/* Banner */}
      {banner && (
        <div className="mx-auto max-w-7xl px-6 pt-4">
          <Alert variant={banner.type === "error" ? "destructive" : "success"}>
            <AlertDescription>{banner.message}</AlertDescription>
          </Alert>
        </div>
      )}

      <main className="mx-auto max-w-7xl space-y-6 px-6 py-8">
        {/* Shared Environment Variables */}
        <Card>
          <CardContent className="pt-6">
            <EnvEditor
              title="共通環境変数"
              description="全てのAIツールで共有される値。PAT やプロキシ設定などはこちらに入力してください。"
              rows={sharedEnv}
              onChange={setSharedEnv}
            />
          </CardContent>
        </Card>

        {/* Tool-specific Environment Variables */}
        <Card>
          <CardHeader className="pb-3">
            <p className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
              Tool-specific
            </p>
            <h3 className="mt-1 text-lg font-semibold">ツール固有の環境変数</h3>
            <p className="mt-2 text-sm text-muted-foreground">
              各ツール固有に上書きしたい値がある場合はこちらから設定します。
              共通設定との競合がある場合はツール設定が優先されます。
            </p>
          </CardHeader>
          <CardContent className="space-y-6">
            {data?.tools?.map((tool) => (
              <div key={tool.id} className="rounded-lg border p-4">
                <EnvEditor
                  title={tool.displayName}
                  description={`${tool.executionType} / ${tool.command}`}
                  rows={toolEnv[tool.id] ?? []}
                  onChange={(rows) =>
                    setToolEnv((prev) => ({ ...prev, [tool.id]: rows }))
                  }
                />
              </div>
            ))}
            {!data?.tools?.length && (
              <p className="py-4 text-center text-sm text-muted-foreground">
                登録されているツールがありません。
              </p>
            )}
          </CardContent>
        </Card>
      </main>
    </div>
  );
}
