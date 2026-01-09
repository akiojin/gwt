import React, { useMemo } from "react";
import { Link } from "react-router-dom";
import { Card, CardHeader, CardContent } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import { Alert, AlertDescription } from "@/components/ui/alert";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { cn } from "@/lib/utils";
import { getAgentTailwindClass } from "@/lib/coding-agent-colors";
import type { ApiCodingAgent, Branch } from "../../../../../types/api.js";

type ToolMode = "normal" | "continue" | "resume";

export type SelectableTool =
  | { id: "claude-code"; label: string; target: "claude" }
  | { id: "codex-cli"; label: string; target: "codex" }
  | { id: string; label: string; target: "custom"; definition: ApiCodingAgent };

interface ToolSummary {
  command: string;
  defaultArgs?: string[] | null;
  modeArgs?: {
    normal?: string[];
    continue?: string[];
    resume?: string[];
  };
  permissionSkipArgs?: string[] | null;
}

interface ToolLauncherProps {
  branch: Branch;
  availableTools: SelectableTool[];
  selectedToolId: string;
  selectedMode: ToolMode;
  skipPermissions: boolean;
  extraArgsText: string;
  isConfigLoading?: boolean;
  configError?: Error | null;
  isStartingSession: boolean;
  isSyncingBranch: boolean;
  needsRemoteSync: boolean;
  hasConflictingDivergence: boolean;
  onToolChange: (toolId: string) => void;
  onModeChange: (mode: ToolMode) => void;
  onSkipPermissionsChange: (skip: boolean) => void;
  onExtraArgsChange: (args: string) => void;
  onStartSession: () => void;
  onSyncBranch: () => void;
}

const BUILTIN_TOOL_SUMMARIES: Record<string, ToolSummary> = {
  "claude-code": {
    command: "claude",
    defaultArgs: [],
    modeArgs: {
      normal: [],
      continue: ["-c"],
      resume: ["-r"],
    },
    permissionSkipArgs: ["--dangerously-skip-permissions"],
  },
  "codex-cli": {
    command: "codex",
    defaultArgs: ["--auto-approve", "--verbose"],
    modeArgs: {
      normal: [],
      continue: ["resume", "--last"],
      resume: ["resume"],
    },
  },
};

export function ToolLauncher({
  branch,
  availableTools,
  selectedToolId,
  selectedMode,
  skipPermissions,
  extraArgsText,
  isConfigLoading,
  configError,
  isStartingSession,
  isSyncingBranch,
  needsRemoteSync,
  hasConflictingDivergence,
  onToolChange,
  onModeChange,
  onSkipPermissionsChange,
  onExtraArgsChange,
  onStartSession,
  onSyncBranch,
}: ToolLauncherProps) {
  const canStartSession = Boolean(branch.worktreePath);
  const selectedTool = availableTools.find((t) => t.id === selectedToolId);

  const selectedToolSummary: ToolSummary | null = useMemo(() => {
    if (!selectedTool) return null;
    if (selectedTool.target === "custom") {
      return {
        command: selectedTool.definition.command,
        defaultArgs: selectedTool.definition.defaultArgs ?? null,
        modeArgs: selectedTool.definition.modeArgs,
        permissionSkipArgs: selectedTool.definition.permissionSkipArgs ?? null,
      };
    }
    return BUILTIN_TOOL_SUMMARIES[selectedTool.id] ?? null;
  }, [selectedTool]);

  const argsPreview = useMemo(() => {
    if (!selectedToolSummary) return null;
    const args: string[] = [];
    if (selectedToolSummary.defaultArgs?.length) {
      args.push(...selectedToolSummary.defaultArgs);
    }
    const mode = selectedToolSummary.modeArgs?.[selectedMode];
    if (mode?.length) args.push(...mode);
    if (skipPermissions && selectedToolSummary.permissionSkipArgs?.length) {
      args.push(...selectedToolSummary.permissionSkipArgs);
    }
    const extraArgs = extraArgsText
      .split(/\s+/)
      .map((c) => c.trim())
      .filter(Boolean);
    if (extraArgs.length) args.push(...extraArgs);
    return { command: selectedToolSummary.command, args };
  }, [selectedToolSummary, selectedMode, skipPermissions, extraArgsText]);

  return (
    <Card>
      <CardHeader className="pb-3">
        <div className="flex items-center justify-between">
          <div>
            <p className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
              Tool Launcher
            </p>
            <h3 className="mt-1 text-lg font-semibold">Coding Agent</h3>
          </div>
          {configError && <Badge variant="warning">設定の取得に失敗</Badge>}
        </div>
        <p className="mt-2 text-sm text-muted-foreground">
          Web UI から直接 Coding Agent
          を起動できます。設定したカスタムエージェントも一覧に表示されます。
        </p>
      </CardHeader>

      <CardContent className="space-y-4">
        {!canStartSession ? (
          <p className="py-4 text-center text-sm text-muted-foreground">
            Worktreeが未作成のため、先にWorktreeを作成してください。
          </p>
        ) : (
          <>
            {/* Form Grid */}
            <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
              <div className="space-y-2">
                <label className="text-sm font-medium">Coding Agent</label>
                <Select
                  value={selectedToolId}
                  onValueChange={onToolChange}
                  disabled={isConfigLoading ?? false}
                >
                  <SelectTrigger>
                    <SelectValue placeholder="Select Coding Agent" />
                  </SelectTrigger>
                  <SelectContent>
                    {availableTools.map((tool) => (
                      <SelectItem
                        key={tool.id}
                        value={tool.id}
                        className={getAgentTailwindClass(tool.id)}
                      >
                        {tool.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>

              <div className="space-y-2">
                <label className="text-sm font-medium">起動モード</label>
                <Select
                  value={selectedMode}
                  onValueChange={(v) => onModeChange(v as ToolMode)}
                >
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="normal">Normal</SelectItem>
                    <SelectItem value="continue">Continue</SelectItem>
                    <SelectItem value="resume">Resume</SelectItem>
                  </SelectContent>
                </Select>
              </div>

              <div className="space-y-2 sm:col-span-2 lg:col-span-1">
                <label className="text-sm font-medium">追加引数</label>
                <Input
                  type="text"
                  value={extraArgsText}
                  onChange={(e) => onExtraArgsChange(e.target.value)}
                  placeholder="--flag value"
                />
              </div>
            </div>

            {/* Skip Permissions Checkbox */}
            <label className="flex items-center gap-2 text-sm">
              <input
                type="checkbox"
                checked={skipPermissions}
                onChange={(e) => onSkipPermissionsChange(e.target.checked)}
                className="h-4 w-4 rounded border-border"
              />
              <span>権限チェックをスキップ (自己責任)</span>
            </label>

            {skipPermissions && (
              <Alert variant="warning">
                <AlertDescription>
                  権限チェックをスキップすることで、CLI での
                  `--dangerously-skip-permissions`
                  指定と同様のリスクを負います。
                </AlertDescription>
              </Alert>
            )}

            {needsRemoteSync && (
              <Alert variant="info" data-testid="sync-required">
                <AlertDescription>
                  <p>
                    リモートに未取得の更新 ({branch.divergence?.behind ?? 0}{" "}
                    commits) があるため、Coding Agent
                    を起動する前に同期してください。
                  </p>
                  <p className="mt-1 text-xs text-muted-foreground">
                    CLI の `git fetch --all` と `git pull --ff-only`
                    と同じ処理を Web UI から実行できます。
                  </p>
                </AlertDescription>
              </Alert>
            )}

            {hasConflictingDivergence && (
              <Alert variant="warning" data-testid="divergence-warning">
                <AlertDescription>
                  <p>
                    リモートとローカルの両方に未解決の差分があります。起動は可能ですが、
                    衝突を避けるために差分の解消を推奨します。
                  </p>
                  <ul className="mt-2 list-inside list-disc text-xs text-muted-foreground">
                    <li>
                      git fetch && git pull --ff-only origin {branch.name}
                    </li>
                    <li>git push origin {branch.name} でローカル進捗を共有</li>
                  </ul>
                </AlertDescription>
              </Alert>
            )}

            {/* Action Buttons */}
            <div className="flex flex-wrap gap-2 pt-2">
              <Button
                onClick={onStartSession}
                disabled={
                  isStartingSession ||
                  !selectedTool ||
                  needsRemoteSync ||
                  isSyncingBranch
                }
              >
                {isStartingSession ? "起動中..." : "セッションを起動"}
              </Button>
              <Button
                variant="secondary"
                onClick={onSyncBranch}
                disabled={!branch.worktreePath || isSyncingBranch}
              >
                {isSyncingBranch ? "同期中..." : "最新の変更を同期"}
              </Button>
              <Button variant="ghost" asChild>
                <Link to="/config">設定を編集</Link>
              </Button>
            </div>

            {/* Command Preview */}
            {selectedToolSummary && (
              <div className="space-y-2 rounded-lg border bg-muted/30 p-4">
                <div className="grid gap-2 text-sm sm:grid-cols-2">
                  <div>
                    <span className="text-muted-foreground">コマンド:</span>{" "}
                    <code className="rounded bg-muted px-1.5 py-0.5 font-mono">
                      {selectedToolSummary.command}
                    </code>
                  </div>
                  <div>
                    <span className="text-muted-foreground">defaultArgs:</span>{" "}
                    <span
                      className={cn(
                        !selectedToolSummary.defaultArgs?.length &&
                          "text-muted-foreground/50",
                      )}
                    >
                      {selectedToolSummary.defaultArgs?.join(" ") || "未設定"}
                    </span>
                  </div>
                </div>
                {argsPreview && (
                  <div className="border-t pt-2">
                    <span className="text-xs text-muted-foreground">
                      最終コマンド:
                    </span>
                    <pre className="mt-1 overflow-x-auto rounded bg-background p-2 font-mono text-sm">
                      {argsPreview.command} {argsPreview.args.join(" ")}
                    </pre>
                  </div>
                )}
              </div>
            )}
          </>
        )}
      </CardContent>
    </Card>
  );
}
