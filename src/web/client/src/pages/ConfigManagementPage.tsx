import React, { useEffect, useMemo, useState } from "react";
import { Link } from "react-router-dom";
import type {
  ConfigPayload,
  CustomAITool,
  EnvironmentVariable,
} from "../../../../types/api.js";
import { useConfig, useUpdateConfig } from "../hooks/useConfig";
import { EnvEditor, createEnvRow, type EnvRow } from "../components/EnvEditor";

type ToolEnvState = Record<string, EnvRow[]>;

function rowsFromVariables(variables?: EnvironmentVariable[] | null): EnvRow[] {
  if (!variables) {
    return [];
  }
  return variables.map((variable) => {
    const partial: Partial<EnvRow> = {
      key: variable.key,
      value: variable.value,
    };

    if (typeof variable.importedFromOs === "boolean") {
      partial.importedFromOs = variable.importedFromOs;
    }
    if (variable.lastUpdated) {
      partial.lastUpdated = variable.lastUpdated;
    }

    return createEnvRow(partial);
  });
}

function serializeRows(rows: EnvRow[]): EnvironmentVariable[] {
  return rows
    .filter((row) => row.key.trim().length > 0)
    .map((row) => ({
      key: row.key.trim().toUpperCase(),
      value: row.value,
    }));
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
    if (serializedOriginalShared !== serializedCurrentShared) {
      return true;
    }
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

  if (isLoading) {
    return (
      <div className="app-shell">
        <div className="page-state page-state--centered">
          <h1>読み込み中</h1>
          <p>設定を読み込んでいます...</p>
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
      <header className="page-hero">
        <Link to="/" className="page-hero__back">
          ← ブランチ一覧へ
        </Link>
        <p className="page-hero__eyebrow">CONFIG</p>
        <h1>環境変数の管理</h1>
        <p className="page-hero__subtitle">
          共通環境変数とツールごとの上書きをブラウザから編集できます。
        </p>
        <div className="page-hero__actions">
          <button
            type="button"
            className="button button--primary"
            onClick={handleSave}
            disabled={updateConfig.isPending || hasInvalidRows || !hasChanges}
          >
            {updateConfig.isPending ? "保存中..." : "保存"}
          </button>
        </div>
        {banner && (
          <div className={`inline-banner inline-banner--${banner.type}`}>
            {banner.message}
          </div>
        )}
      </header>

      <main className="page-content page-content--wide">
        <section className="section-card">
          <EnvEditor
            title="共通環境変数"
            description="全てのAIツールで共有される値。PAT やプロキシ設定などはこちらに入力してください。"
            rows={sharedEnv}
            onChange={setSharedEnv}
          />
        </section>

        <section className="section-card">
          <h2>ツール固有の環境変数</h2>
          <p className="section-card__body">
            各ツール固有に上書きしたい値がある場合はこちらから設定します。共通設定との競合がある場合は
            ツール設定が優先されます。
          </p>
          <div className="env-editor__tool-list">
            {data?.tools?.map((tool) => (
              <div key={tool.id} className="env-editor__tool">
                <EnvEditor
                  title={tool.displayName}
                  description={`${tool.executionType} / ${tool.command}`}
                  rows={toolEnv[tool.id] ?? []}
                  onChange={(rows) =>
                    setToolEnv((prev) => ({
                      ...prev,
                      [tool.id]: rows,
                    }))
                  }
                />
              </div>
            ))}
          </div>
        </section>
      </main>
    </div>
  );
}
