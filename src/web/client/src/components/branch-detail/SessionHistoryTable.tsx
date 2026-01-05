import React from "react";
import { Card, CardHeader, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Table,
  TableHeader,
  TableBody,
  TableRow,
  TableHead,
  TableCell,
} from "@/components/ui/table";

interface SessionInfo {
  sessionId: string;
  worktreePath: string;
  agentType: string;
  agentName?: string | null;
  mode?: string;
  status: "pending" | "running" | "completed" | "failed";
  startedAt?: string;
  endedAt?: string | null;
}

interface SessionHistoryTableProps {
  sessions: SessionInfo[];
  isLoading?: boolean;
  terminatingSessionId: string | null;
  isDeleting?: boolean;
  onTerminate: (sessionId: string) => void;
  onSelectSession: (sessionId: string) => void;
}

const SESSION_STATUS_VARIANT: Record<
  SessionInfo["status"],
  "default" | "success" | "warning" | "destructive" | "outline"
> = {
  pending: "outline",
  running: "success",
  completed: "default",
  failed: "destructive",
};

const SESSION_STATUS_LABEL: Record<SessionInfo["status"], string> = {
  pending: "Pending",
  running: "Running",
  completed: "Completed",
  failed: "Failed",
};

function formatDate(value?: string | null): string {
  if (!value) return "--";
  try {
    return new Intl.DateTimeFormat("ja-JP", {
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    }).format(new Date(value));
  } catch {
    return value;
  }
}

function agentLabel(agentType: string, agentName?: string | null): string {
  if (agentType === "custom") return agentName ?? "Custom";
  if (agentType === "codex-cli") return "Codex CLI";
  return "Claude Code";
}

export function SessionHistoryTable({
  sessions,
  isLoading,
  terminatingSessionId,
  isDeleting,
  onTerminate,
  onSelectSession,
}: SessionHistoryTableProps) {
  return (
    <Card>
      <CardHeader className="pb-3">
        <div className="flex items-center justify-between">
          <div>
            <p className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
              Session History
            </p>
            <h3 className="mt-1 text-lg font-semibold">セッション履歴</h3>
          </div>
          {isLoading && (
            <Badge variant="outline" className="animate-pulse">
              読み込み中...
            </Badge>
          )}
        </div>
        <p className="mt-2 text-sm text-muted-foreground">
          この Worktree に紐づいた Coding Agent セッション履歴です。CLI
          からの起動分も共有されます。
        </p>
      </CardHeader>
      <CardContent>
        {sessions.length === 0 ? (
          <p className="py-8 text-center text-sm text-muted-foreground">
            セッション履歴はまだありません。
          </p>
        ) : (
          <div className="overflow-x-auto rounded-md border">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead className="w-24">状態</TableHead>
                  <TableHead>ツール</TableHead>
                  <TableHead className="w-20">モード</TableHead>
                  <TableHead>開始</TableHead>
                  <TableHead>終了</TableHead>
                  <TableHead className="w-24 text-right">操作</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {sessions.slice(0, 5).map((session) => (
                  <TableRow
                    key={session.sessionId}
                    className="cursor-pointer hover:bg-muted/50"
                    onClick={() => {
                      if (session.status === "running") {
                        onSelectSession(session.sessionId);
                      }
                    }}
                  >
                    <TableCell>
                      <Badge variant={SESSION_STATUS_VARIANT[session.status]}>
                        {SESSION_STATUS_LABEL[session.status]}
                      </Badge>
                    </TableCell>
                    <TableCell className="font-medium">
                      {agentLabel(session.agentType, session.agentName)}
                    </TableCell>
                    <TableCell>
                      <span className="text-muted-foreground">
                        {session.mode}
                      </span>
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {formatDate(session.startedAt)}
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {formatDate(session.endedAt)}
                    </TableCell>
                    <TableCell className="text-right">
                      {session.status === "running" ? (
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={(e) => {
                            e.stopPropagation();
                            onTerminate(session.sessionId);
                          }}
                          disabled={
                            terminatingSessionId === session.sessionId ||
                            isDeleting
                          }
                        >
                          {terminatingSessionId === session.sessionId
                            ? "終了中..."
                            : "終了"}
                        </Button>
                      ) : (
                        <span className="text-sm text-muted-foreground">
                          --
                        </span>
                      )}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        )}
      </CardContent>
    </Card>
  );
}
