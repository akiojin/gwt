import React from "react";
import {
  Card,
  CardHeader,
  CardContent,
  CardFooter,
} from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import type { ApiCodingAgent } from "../../../../types/api.js";

interface CustomCodingAgentListProps {
  agents: ApiCodingAgent[];
  onEdit: (agent: ApiCodingAgent) => void;
  onDelete: (agent: ApiCodingAgent) => void;
}

export function CustomCodingAgentList({
  agents,
  onEdit,
  onDelete,
}: CustomCodingAgentListProps) {
  if (!agents.length) {
    return (
      <Card className="border-dashed">
        <CardContent className="flex flex-col items-center justify-center py-12 text-center">
          <h3 className="text-lg font-semibold">
            Custom Coding Agent が登録されていません
          </h3>
          <p className="mt-2 text-sm text-muted-foreground">
            「Coding Agent を追加」から最初のエージェントを登録してください。
          </p>
        </CardContent>
      </Card>
    );
  }

  return (
    <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
      {agents.map((agent) => (
        <Card key={agent.id} className="flex flex-col">
          <CardHeader className="pb-3">
            <div className="flex items-start justify-between gap-2">
              <div className="min-w-0 flex-1">
                <p className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
                  ID: {agent.id}
                </p>
                <h3 className="mt-1 flex items-center gap-2 font-semibold">
                  {agent.icon && <span>{agent.icon}</span>}
                  <span className="truncate">{agent.displayName}</span>
                </h3>
              </div>
              <Badge variant="outline">
                {renderExecutionLabel(agent.executionType)}
              </Badge>
            </div>
          </CardHeader>

          <CardContent className="flex-1 space-y-3 pb-3">
            <p className="rounded bg-muted px-2 py-1 font-mono text-sm">
              {agent.command}
            </p>

            {agent.description && (
              <p className="text-sm text-muted-foreground">
                {agent.description}
              </p>
            )}

            <dl className="grid grid-cols-3 gap-2 text-xs">
              <div>
                <dt className="text-muted-foreground">normal</dt>
                <dd className="mt-0.5">{renderArgs(agent.modeArgs?.normal)}</dd>
              </div>
              <div>
                <dt className="text-muted-foreground">continue</dt>
                <dd className="mt-0.5">
                  {renderArgs(agent.modeArgs?.continue)}
                </dd>
              </div>
              <div>
                <dt className="text-muted-foreground">resume</dt>
                <dd className="mt-0.5">{renderArgs(agent.modeArgs?.resume)}</dd>
              </div>
            </dl>
          </CardContent>

          <CardFooter className="flex gap-2 pt-0">
            <Button variant="secondary" size="sm" onClick={() => onEdit(agent)}>
              編集
            </Button>
            <Button variant="ghost" size="sm" onClick={() => onDelete(agent)}>
              削除
            </Button>
          </CardFooter>
        </Card>
      ))}
    </div>
  );
}

function renderExecutionLabel(type: ApiCodingAgent["executionType"]) {
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
    return <span className="text-muted-foreground/50">未設定</span>;
  }
  return args.join(" ");
}
