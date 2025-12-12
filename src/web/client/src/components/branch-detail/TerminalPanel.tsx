import React from "react";
import { Card, CardHeader, CardContent } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Terminal } from "../Terminal";
import { cn } from "@/lib/utils";

interface TerminalPanelProps {
  sessionId: string | null;
  isFullscreen: boolean;
  onToggleFullscreen: () => void;
  onExit: (code: number) => void;
  onError: (message: string | null) => void;
}

export function TerminalPanel({
  sessionId,
  isFullscreen,
  onToggleFullscreen,
  onExit,
  onError,
}: TerminalPanelProps) {
  if (!sessionId) {
    return (
      <Card className="h-full">
        <CardHeader className="pb-2">
          <p className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
            Terminal
          </p>
          <h3 className="text-lg font-semibold">セッションは未起動</h3>
        </CardHeader>
        <CardContent className="flex min-h-[200px] items-center justify-center">
          <p className="text-center text-sm text-muted-foreground">
            上部のアクションからAIツールを起動すると、<br />
            このエリアにターミナルが表示されます。
          </p>
        </CardContent>
      </Card>
    );
  }

  return (
    <Card
      className={cn(
        "flex flex-col transition-all",
        isFullscreen && "fixed inset-4 z-50 h-auto"
      )}
      data-testid="active-terminal"
    >
      <CardHeader className="flex-shrink-0 pb-2">
        <div className="flex items-center justify-between">
          <div>
            <p className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
              Active Session
            </p>
            <h3 className="text-lg font-semibold">ターミナルセッション</h3>
          </div>
          <div className="flex gap-2">
            <Button variant="ghost" size="sm" onClick={onToggleFullscreen}>
              {isFullscreen ? "通常表示に戻す" : "最大化"}
            </Button>
            {isFullscreen && (
              <Button
                variant="ghost"
                size="sm"
                onClick={onToggleFullscreen}
                aria-label="ターミナルを閉じる"
              >
                ×
              </Button>
            )}
          </div>
        </div>
        <p className="text-sm text-muted-foreground">
          出力はリアルタイムにストリームされます。終了するとこのパネルは自動で閉じます。
        </p>
      </CardHeader>
      <CardContent className={cn("flex-1 overflow-hidden", isFullscreen && "h-full")}>
        <div
          className={cn(
            "h-full min-h-[300px] overflow-auto rounded-lg border bg-black p-4",
            isFullscreen && "min-h-0"
          )}
        >
          <Terminal
            sessionId={sessionId}
            onExit={onExit}
            onError={onError}
          />
        </div>
      </CardContent>
    </Card>
  );
}
