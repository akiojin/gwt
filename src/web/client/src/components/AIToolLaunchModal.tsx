import React, { useEffect, useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
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
import type { Branch, CustomAITool } from "../../../../types/api.js";
import {
  CLAUDE_PERMISSION_SKIP_ARGS,
  CODEX_DEFAULT_ARGS,
} from "../../../../shared/aiToolConstants.js";
import { useConfig } from "../hooks/useConfig";
import { useStartSession } from "../hooks/useSessions";
import { useCreateWorktree } from "../hooks/useWorktrees";
import { useSyncBranch } from "../hooks/useBranches";
import { ApiError } from "../lib/api";

const BUILTIN_TOOL_SUMMARIES: Record<string, ToolSummary> = {
  "claude-code": {
    command: "claude",
    defaultArgs: [],
    modeArgs: {
      normal: [],
      continue: ["-c"],
      resume: ["-r"],
    },
    permissionSkipArgs: Array.from(CLAUDE_PERMISSION_SKIP_ARGS),
  },
  "codex-cli": {
    command: "codex",
    defaultArgs: Array.from(CODEX_DEFAULT_ARGS),
    modeArgs: {
      normal: [],
      continue: ["resume", "--last"],
      resume: ["resume"],
    },
  },
};

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

interface AIToolLaunchModalProps {
  branch: Branch;
  onClose: () => void;
}

type ToolMode = "normal" | "continue" | "resume";

type SelectableTool =
  | { id: "claude-code"; label: string; target: "claude" }
  | { id: "codex-cli"; label: string; target: "codex" }
  | { id: string; label: string; target: "custom"; definition: CustomAITool };

export function AIToolLaunchModal({ branch, onClose }: AIToolLaunchModalProps) {
  const { data: config, isLoading: isConfigLoading, error: configError } = useConfig();
  const startSession = useStartSession();
  const createWorktree = useCreateWorktree();
  const syncBranch = useSyncBranch(branch.name);
  const navigate = useNavigate();

  const [selectedToolId, setSelectedToolId] = useState<string>("claude-code");
  const [selectedMode, setSelectedMode] = useState<ToolMode>("normal");
  const [skipPermissions, setSkipPermissions] = useState(false);
  const [extraArgsText, setExtraArgsText] = useState("");
  const [banner, setBanner] = useState<{ type: "success" | "error" | "info"; message: string } | null>(null);
  const [isStartingSession, setIsStartingSession] = useState(false);
  const [isCreatingWorktree, setIsCreatingWorktree] = useState(false);

  const customTools = config?.tools ?? [];
  const availableTools: SelectableTool[] = useMemo(
    () => [
      { id: "claude-code", label: "Claude Code", target: "claude" },
      { id: "codex-cli", label: "Codex CLI", target: "codex" },
      ...customTools.map((tool) => ({
        id: tool.id,
        label: tool.displayName,
        target: "custom" as const,
        definition: tool,
      })),
    ],
    [customTools],
  );

  useEffect(() => {
    if (!availableTools.length) {
      setSelectedToolId("claude-code");
      return;
    }
    if (!availableTools.find((tool) => tool.id === selectedToolId)) {
      const first = availableTools[0];
      if (first) {
        setSelectedToolId(first.id);
      }
    }
  }, [availableTools, selectedToolId]);

  const selectedTool = availableTools.find((tool) => tool.id === selectedToolId);

  const selectedToolSummary: ToolSummary | null = useMemo(() => {
    if (!selectedTool) {
      return null;
    }
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
    if (!selectedToolSummary) {
      return null;
    }
    const args: string[] = [];
    if (selectedToolSummary.defaultArgs?.length) {
      args.push(...selectedToolSummary.defaultArgs);
    }
    const mode = selectedToolSummary.modeArgs?.[selectedMode];
    if (mode?.length) {
      args.push(...mode);
    }
    if (skipPermissions && selectedToolSummary.permissionSkipArgs?.length) {
      args.push(...selectedToolSummary.permissionSkipArgs);
    }
    const extraArgs = parseExtraArgs(extraArgsText);
    if (extraArgs.length) {
      args.push(...extraArgs);
    }
    return { command: selectedToolSummary.command, args };
  }, [selectedToolSummary, selectedMode, skipPermissions, extraArgsText]);

  const PROTECTED_BRANCHES = ["main", "master", "develop"];
  const isProtectedBranch = PROTECTED_BRANCHES.includes(
    branch.name.replace(/^origin\//, ""),
  );
  const divergenceInfo = branch.divergence ?? null;
  const hasBlockingDivergence = Boolean(
    divergenceInfo && divergenceInfo.ahead > 0 && divergenceInfo.behind > 0,
  );
  const needsRemoteSync = Boolean(
    branch.worktreePath &&
      divergenceInfo &&
      divergenceInfo.behind > 0 &&
      divergenceInfo.ahead === 0 &&
      !hasBlockingDivergence,
  );

  useEffect(() => {
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = previousOverflow;
    };
  }, []);

  const handleClose = () => {
    setBanner(null);
    onClose();
  };

  const handleCreateWorktree = async () => {
    if (isProtectedBranch) {
      setBanner({
        type: "error",
        message: `Cannot create worktree for protected branch: ${branch.name}. Protected branches (main, develop, master) must remain in the main repository.`,
      });
      return;
    }
    setIsCreatingWorktree(true);
    try {
      await createWorktree.mutateAsync({ branchName: branch.name, createBranch: false });
      setBanner({ type: "success", message: `Worktree created for ${branch.name}. Please sync before launching.` });
    } catch (error) {
      setBanner({ type: "error", message: formatError(error, "Failed to create worktree") });
    } finally {
      setIsCreatingWorktree(false);
    }
  };

  const handleSyncBranch = async () => {
    if (!branch.worktreePath) {
      setBanner({ type: "error", message: "Cannot sync because worktree is missing." });
      return;
    }
    try {
      const result = await syncBranch.mutateAsync({ worktreePath: branch.worktreePath });
      if (result.pullStatus === "success") {
        setBanner({ type: "success", message: "Fetched latest changes from remote." });
      } else {
        const warning = result.warnings?.join("\n") ?? "fast-forward pull did not complete.";
        setBanner({ type: "error", message: `git pull --ff-only failed.\n${warning}` });
      }
    } catch (error) {
      setBanner({ type: "error", message: formatError(error, "Git sync failed") });
    }
  };

  const handleStartSession = async () => {
    if (!branch.worktreePath) {
      setBanner({ type: "error", message: "Worktree missing. Create one first." });
      return;
    }
    if (!selectedTool) {
      setBanner({ type: "error", message: "Select an AI tool to launch." });
      return;
    }
    if (needsRemoteSync) {
      setBanner({ type: "error", message: "Cannot launch until remote updates are synced." });
      return;
    }
    if (hasBlockingDivergence) {
      setBanner({
        type: "error",
        message: "Both remote and local have diverged. Resolve differences before launching.",
      });
      return;
    }

    if (skipPermissions && !window.confirm("Skip permission checks? This is risky.")) {
      return;
    }

    setIsStartingSession(true);
    try {
      const toolType =
        selectedTool.target === "codex"
          ? "codex-cli"
          : selectedTool.target === "custom"
            ? "custom"
            : "claude-code";
      const extraArgs = parseExtraArgs(extraArgsText);
      const sessionRequest = {
        toolType,
        toolName: selectedTool.target === "custom" ? selectedTool.id : null,
        ...(selectedTool.target === "custom" ? { customToolId: selectedTool.id } : {}),
        mode: selectedMode,
        worktreePath: branch.worktreePath,
        skipPermissions,
        ...(selectedTool.target === "codex" ? { bypassApprovals: skipPermissions } : {}),
        ...(extraArgs.length ? { extraArgs } : {}),
      } as const;

      const session = await startSession.mutateAsync(sessionRequest);
      handleClose();
      navigate(`/${encodeURIComponent(branch.name)}`, {
        state: { focusSessionId: session.sessionId },
      });
    } catch (error) {
      setBanner({ type: "error", message: formatError(error, "Failed to start session") });
    } finally {
      setIsStartingSession(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/80" role="dialog" aria-modal="true">
      <Card className="mx-4 w-full max-w-2xl" role="document">
        <CardHeader className="pb-4">
          <div className="flex items-start justify-between gap-4">
            <div>
              <p className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
                Launch AI Tool
              </p>
              <h2 className="mt-1 text-lg font-semibold">{branch.name}</h2>
            </div>
            <Button variant="ghost" size="sm" onClick={handleClose}>
              ×
            </Button>
          </div>
        </CardHeader>

        <CardContent className="space-y-4">
          {banner && (
            <Alert variant={banner.type === "error" ? "destructive" : banner.type === "success" ? "success" : "info"}>
              <AlertDescription>{banner.message}</AlertDescription>
            </Alert>
          )}

          {configError && (
            <Alert variant="warning">
              <AlertDescription>
                Failed to load config: {configError instanceof Error ? configError.message : "unknown"}
              </AlertDescription>
            </Alert>
          )}

          {!branch.worktreePath && (
            <Alert variant={isProtectedBranch ? "destructive" : "warning"}>
              <AlertDescription className="space-y-2">
                {isProtectedBranch ? (
                  <p>
                    Cannot create worktree for protected branches (main, develop, master).
                    Protected branches must remain in the main repository.
                  </p>
                ) : (
                  <>
                    <p>Worktree is missing. Create it before launching AI tools.</p>
                    <Button
                      variant="secondary"
                      size="sm"
                      onClick={handleCreateWorktree}
                      disabled={isCreatingWorktree}
                    >
                      {isCreatingWorktree ? "Creating..." : "Create worktree"}
                    </Button>
                  </>
                )}
              </AlertDescription>
            </Alert>
          )}

          {needsRemoteSync && (
            <Alert variant="info">
              <AlertDescription>
                Remote has {branch.divergence?.behind ?? 0} commits you need to pull before launching.
              </AlertDescription>
            </Alert>
          )}

          {hasBlockingDivergence && (
            <Alert variant="warning">
              <AlertDescription>
                Both remote and local have unresolved differences. Rebase/merge before launching.
              </AlertDescription>
            </Alert>
          )}

          <div className="grid gap-4 sm:grid-cols-3">
            <div className="space-y-2">
              <label className="text-sm font-medium">AI tool</label>
              <Select
                value={selectedToolId}
                onValueChange={setSelectedToolId}
                disabled={isConfigLoading ?? false}
              >
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {availableTools.map((tool) => (
                    <SelectItem key={tool.id} value={tool.id}>
                      {tool.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>

            <div className="space-y-2">
              <label className="text-sm font-medium">Launch mode</label>
              <Select
                value={selectedMode}
                onValueChange={(value) => setSelectedMode(value as ToolMode)}
              >
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="normal">normal</SelectItem>
                  <SelectItem value="continue">continue</SelectItem>
                  <SelectItem value="resume">resume</SelectItem>
                </SelectContent>
              </Select>
            </div>

            <div className="space-y-2">
              <label className="text-sm font-medium">Extra args</label>
              <Input
                type="text"
                value={extraArgsText}
                onChange={(event) => setExtraArgsText(event.target.value)}
                placeholder="--flag value"
              />
            </div>
          </div>

          <label className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={skipPermissions}
              onChange={(event) => setSkipPermissions(event.target.checked)}
              className="h-4 w-4 rounded border-border"
            />
            <span>Skip permission checks (at your own risk)</span>
          </label>

          <div className="flex flex-wrap gap-2">
            <Button
              onClick={handleStartSession}
              disabled={isStartingSession || !selectedTool || hasBlockingDivergence || needsRemoteSync}
            >
              {isStartingSession ? "Launching..." : "Launch AI tool"}
            </Button>
            <Button
              variant="secondary"
              onClick={handleSyncBranch}
              disabled={!branch.worktreePath || syncBranch.isPending}
            >
              {syncBranch.isPending ? "Syncing..." : "Sync latest"}
            </Button>
            <Button variant="ghost" onClick={handleClose}>
              Cancel
            </Button>
          </div>

          {selectedToolSummary && (
            <div className="space-y-2 rounded-lg border bg-muted/30 p-4 text-sm">
              <div className="grid gap-2 sm:grid-cols-3">
                <div>
                  <span className="text-muted-foreground">Command:</span>{" "}
                  <code className="rounded bg-muted px-1.5 py-0.5 font-mono">
                    {selectedToolSummary.command}
                  </code>
                </div>
                <div>
                  <span className="text-muted-foreground">defaultArgs:</span>{" "}
                  <span className={!selectedToolSummary.defaultArgs?.length ? "text-muted-foreground/50" : ""}>
                    {renderArgs(selectedToolSummary.defaultArgs)}
                  </span>
                </div>
                <div>
                  <span className="text-muted-foreground">permissionSkipArgs:</span>{" "}
                  <span className={!selectedToolSummary.permissionSkipArgs?.length ? "text-muted-foreground/50" : ""}>
                    {renderArgs(selectedToolSummary.permissionSkipArgs)}
                  </span>
                </div>
              </div>
              {argsPreview && (
                <div className="border-t pt-2">
                  <span className="text-xs text-muted-foreground">Command to run:</span>
                  <pre className="mt-1 overflow-x-auto rounded bg-background p-2 font-mono text-sm">
                    {argsPreview.command} {argsPreview.args.join(" ")}
                  </pre>
                </div>
              )}
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  );
}

function parseExtraArgs(value: string): string[] {
  return value
    .split(/\s+/)
    .map((chunk) => chunk.trim())
    .filter(Boolean);
}

function renderArgs(args?: string[] | null) {
  if (!args || args.length === 0) {
    return "未設定";
  }
  return args.join(" ");
}

function formatError(error: unknown, fallback: string) {
  if (error instanceof ApiError) {
    return `${error.message}${error.details ? `\n${error.details}` : ""}`;
  }
  if (error instanceof Error) {
    return error.message;
  }
  return fallback;
}
