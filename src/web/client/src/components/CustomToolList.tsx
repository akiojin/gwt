import React from "react";
import { Card, CardHeader, CardContent, CardFooter } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import type { CustomAITool } from "../../../../types/api.js";

interface CustomToolListProps {
  tools: CustomAITool[];
  onEdit: (tool: CustomAITool) => void;
  onDelete: (tool: CustomAITool) => void;
}

export function CustomToolList({ tools, onEdit, onDelete }: CustomToolListProps) {
  if (!tools.length) {
    return (
      <Card className="border-dashed">
        <CardContent className="flex flex-col items-center justify-center py-12 text-center">
          <h3 className="text-lg font-semibold">カスタムツールが登録されていません</h3>
          <p className="mt-2 text-sm text-muted-foreground">
            「カスタムツールを追加」から最初のツールを登録してください。
          </p>
        </CardContent>
      </Card>
    );
  }

  return (
    <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
      {tools.map((tool) => (
        <Card key={tool.id} className="flex flex-col">
          <CardHeader className="pb-3">
            <div className="flex items-start justify-between gap-2">
              <div className="min-w-0 flex-1">
                <p className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
                  ID: {tool.id}
                </p>
                <h3 className="mt-1 flex items-center gap-2 font-semibold">
                  {tool.icon && <span>{tool.icon}</span>}
                  <span className="truncate">{tool.displayName}</span>
                </h3>
              </div>
              <Badge variant="outline">{renderExecutionLabel(tool.executionType)}</Badge>
            </div>
          </CardHeader>

          <CardContent className="flex-1 space-y-3 pb-3">
            <p className="rounded bg-muted px-2 py-1 font-mono text-sm">
              {tool.command}
            </p>

            {tool.description && (
              <p className="text-sm text-muted-foreground">{tool.description}</p>
            )}

            <dl className="grid grid-cols-3 gap-2 text-xs">
              <div>
                <dt className="text-muted-foreground">normal</dt>
                <dd className="mt-0.5">{renderArgs(tool.modeArgs?.normal)}</dd>
              </div>
              <div>
                <dt className="text-muted-foreground">continue</dt>
                <dd className="mt-0.5">{renderArgs(tool.modeArgs?.continue)}</dd>
              </div>
              <div>
                <dt className="text-muted-foreground">resume</dt>
                <dd className="mt-0.5">{renderArgs(tool.modeArgs?.resume)}</dd>
              </div>
            </dl>
          </CardContent>

          <CardFooter className="flex gap-2 pt-0">
            <Button variant="secondary" size="sm" onClick={() => onEdit(tool)}>
              編集
            </Button>
            <Button variant="ghost" size="sm" onClick={() => onDelete(tool)}>
              削除
            </Button>
          </CardFooter>
        </Card>
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
    return <span className="text-muted-foreground/50">未設定</span>;
  }
  return args.join(" ");
}
