import React from "react";
import { Card, CardHeader, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import type { Branch, LastToolUsage } from "../../../../../types/api.js";

interface BranchInfoCardsProps {
  branch: Branch;
  formattedCommitDate: string;
  latestToolUsage: LastToolUsage | null;
}

function mapToolLabel(toolId: string, toolLabel?: string | null): string {
  if (toolId === "claude-code") return "Claude";
  if (toolId === "codex-cli") return "Codex";
  if (toolId === "gemini-cli") return "Gemini";
  if (toolId === "qwen-cli") return "Qwen";
  if (toolLabel) return toolLabel;
  return "Custom";
}

function renderToolUsage(usage: LastToolUsage): string {
  const modeLabel =
    usage.mode === "normal"
      ? "New"
      : usage.mode === "continue"
        ? "Continue"
        : usage.mode === "resume"
          ? "Resume"
          : null;
  const toolText = mapToolLabel(usage.toolId, usage.toolLabel);
  return [toolText, modeLabel, usage.model].filter(Boolean).join(" | ");
}

function formatUsageTimestamp(value: number): string {
  try {
    return new Intl.DateTimeFormat("ja-JP", {
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    }).format(new Date(value));
  } catch {
    return "--";
  }
}

export function BranchInfoCards({
  branch,
  formattedCommitDate,
  latestToolUsage,
}: BranchInfoCardsProps) {
  return (
    <div className="grid gap-4 lg:grid-cols-2">
      {/* Branch Insights */}
      <Card>
        <CardHeader className="pb-2">
          <p className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
            Branch Insights
          </p>
          <h3 className="text-lg font-semibold">ブランチインサイト</h3>
        </CardHeader>
        <CardContent>
          <dl className="grid gap-3 text-sm">
            <div className="flex justify-between border-b border-border/50 pb-2">
              <dt className="text-muted-foreground">コミット</dt>
              <dd className="font-mono text-xs">{branch.commitHash}</dd>
            </div>
            <div className="flex justify-between border-b border-border/50 pb-2">
              <dt className="text-muted-foreground">Author</dt>
              <dd>{branch.author ?? "N/A"}</dd>
            </div>
            <div className="flex justify-between border-b border-border/50 pb-2">
              <dt className="text-muted-foreground">更新日</dt>
              <dd>{formattedCommitDate}</dd>
            </div>
            <div className="flex justify-between">
              <dt className="text-muted-foreground">Worktree</dt>
              <dd
                className="max-w-[200px] truncate text-right"
                title={branch.worktreePath ?? undefined}
              >
                {branch.worktreePath ?? "未作成"}
              </dd>
            </div>
          </dl>
        </CardContent>
      </Card>

      {/* Commit Message */}
      <Card>
        <CardHeader className="pb-2">
          <p className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
            Latest Commit
          </p>
          <h3 className="text-lg font-semibold">コミット情報</h3>
        </CardHeader>
        <CardContent>
          <p className="text-sm text-muted-foreground">
            {branch.commitMessage ?? "コミットメッセージがありません。"}
          </p>
        </CardContent>
      </Card>

      {/* Divergence Status */}
      {branch.divergence && (
        <Card>
          <CardHeader className="pb-2">
            <p className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
              Divergence
            </p>
            <h3 className="text-lg font-semibold">差分状況</h3>
          </CardHeader>
          <CardContent>
            <div className="flex flex-wrap gap-2">
              <Badge variant="outline">↑ Ahead {branch.divergence.ahead}</Badge>
              <Badge variant="outline">
                ↓ Behind {branch.divergence.behind}
              </Badge>
              <Badge
                variant={branch.divergence.upToDate ? "success" : "warning"}
              >
                {branch.divergence.upToDate ? "最新" : "更新あり"}
              </Badge>
            </div>
          </CardContent>
        </Card>
      )}

      {/* Worktree Info */}
      <Card>
        <CardHeader className="pb-2">
          <p className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
            Worktree
          </p>
          <h3 className="text-lg font-semibold">Worktree情報</h3>
        </CardHeader>
        <CardContent className="space-y-2 text-sm text-muted-foreground">
          <p>
            パス:{" "}
            <strong className="text-foreground">
              {branch.worktreePath ?? "未作成"}
            </strong>
          </p>
          <ul className="list-inside list-disc space-y-1 text-xs">
            <li>AIツールの起動にはクリーンなワークツリーであることを推奨</li>
            <li>Worktree再作成で既存のローカル変更が失われる可能性あり</li>
          </ul>
        </CardContent>
      </Card>

      {/* Last Tool Usage */}
      {latestToolUsage && (
        <Card className="lg:col-span-2">
          <CardHeader className="pb-2">
            <p className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
              Last Activity
            </p>
            <h3 className="text-lg font-semibold">最終ツール使用</h3>
          </CardHeader>
          <CardContent>
            <div className="flex flex-wrap items-center gap-2 text-sm">
              <Badge variant="outline">
                {renderToolUsage(latestToolUsage)}
              </Badge>
              <span className="text-muted-foreground">
                {formatUsageTimestamp(latestToolUsage.timestamp)}
              </span>
              {latestToolUsage.worktreePath && (
                <span className="text-xs text-muted-foreground">
                  @ {latestToolUsage.worktreePath}
                </span>
              )}
            </div>
          </CardContent>
        </Card>
      )}
    </div>
  );
}
