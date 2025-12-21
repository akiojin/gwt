/**
 * ブランチノードコンポーネント
 *
 * ソーマ（神経細胞体）をイメージした生物発光ノード
 * 多層グロー効果と有機的なアニメーションで生命感を表現
 */

import React, { memo, useMemo } from "react";
import { Handle, Position, type NodeProps } from "@xyflow/react";
import { cn } from "@/lib/utils";
import { getNodeColor, getNodeSize, type GraphNode } from "./graphUtils";

type BranchNodeProps = NodeProps<GraphNode>;

export const BranchNode = memo(function BranchNode({
  data,
  selected,
}: BranchNodeProps) {
  const { branch } = data;

  if (!branch) return null;

  const size = getNodeSize(branch);
  const _baseColor = getNodeColor(branch);
  const hasWorktree = Boolean(branch.worktreePath);
  const isMerged = branch.mergeStatus === "merged";

  // ブランチ名を短縮
  const displayName = useMemo(() => {
    if (branch.name.length > 18) {
      return `${branch.name.slice(0, 15)}...`;
    }
    return branch.name;
  }, [branch.name]);

  // ノードカラーをHSL値として抽出（動的スタイル用）
  const colorStyle = useMemo(() => {
    if (hasWorktree) {
      return {
        primary: "hsl(160 90% 45%)",
        glow: "hsl(160 90% 45% / 0.4)",
        inner: "hsl(160 90% 50% / 0.2)",
      };
    }
    if (branch.type === "local") {
      return {
        primary: "hsl(280 85% 65%)",
        glow: "hsl(280 85% 65% / 0.4)",
        inner: "hsl(280 85% 70% / 0.2)",
      };
    }
    return {
      primary: "hsl(200 90% 55%)",
      glow: "hsl(200 90% 55% / 0.4)",
      inner: "hsl(200 90% 60% / 0.2)",
    };
  }, [hasWorktree, branch.type]);

  return (
    <>
      {/* 入力ハンドル - デンドライト接続点 */}
      <Handle
        type="target"
        position={Position.Top}
        className="!h-3 !w-3 !rounded-full !border-2 !bg-transparent transition-all duration-300"
        style={{
          borderColor: colorStyle.primary,
          boxShadow: `0 0 8px ${colorStyle.glow}`,
        }}
      />

      {/* ソーマ本体 */}
      <div
        className={cn(
          "group relative flex cursor-pointer items-center justify-center rounded-full",
          "transition-all duration-500 ease-out",
          hasWorktree && "animate-active-synapse",
          !hasWorktree && "animate-soma-pulse",
          selected && "ring-2 ring-offset-4 ring-offset-background",
          isMerged && "opacity-50",
        )}
        style={{
          width: size,
          height: size,
          background: `radial-gradient(circle at 30% 30%, ${colorStyle.inner}, transparent 60%)`,
          border: `2px solid ${colorStyle.primary}`,
          boxShadow: selected
            ? `0 0 30px ${colorStyle.glow}, inset 0 0 20px ${colorStyle.inner}, 0 0 0 4px ${colorStyle.primary}`
            : undefined,
        }}
      >
        {/* 外殻グロー */}
        <div
          className="absolute -inset-2 rounded-full opacity-30 blur-md"
          style={{ backgroundColor: colorStyle.primary }}
        />

        {/* 核膜 - 内側のリング */}
        <div
          className="absolute inset-2 rounded-full border opacity-40"
          style={{ borderColor: colorStyle.primary }}
        />

        {/* 核 - 中心部 */}
        <div
          className="relative z-10 flex items-center justify-center rounded-full shadow-lg"
          style={{
            width: size * 0.45,
            height: size * 0.45,
            background: `linear-gradient(135deg, ${colorStyle.primary}, ${colorStyle.inner})`,
            boxShadow: `0 0 15px ${colorStyle.glow}`,
          }}
        >
          {/* アイコン/タイプ表示 */}
          <span className="text-xs font-bold text-background">
            {branch.type === "local" ? "L" : "R"}
          </span>
        </div>

        {/* 細胞小器官 - 装飾ドット */}
        {!isMerged && (
          <>
            <div
              className="absolute h-1.5 w-1.5 rounded-full animate-vesicle-release"
              style={{
                backgroundColor: colorStyle.primary,
                top: "20%",
                right: "25%",
                animationDelay: "0s",
              }}
            />
            <div
              className="absolute h-1 w-1 rounded-full animate-vesicle-release"
              style={{
                backgroundColor: colorStyle.primary,
                bottom: "25%",
                left: "20%",
                animationDelay: "0.7s",
              }}
            />
          </>
        )}

        {/* Worktree活性インジケータ */}
        {hasWorktree && (
          <div
            className="absolute -right-1 -top-1 flex h-4 w-4 items-center justify-center rounded-full animate-synapse-spark"
            style={{
              backgroundColor: "hsl(160 90% 45%)",
              boxShadow: "0 0 10px hsl(160 90% 45% / 0.8)",
            }}
          >
            <span className="text-[8px] font-bold text-background">W</span>
          </div>
        )}

        {/* マージ済みオーバーレイ */}
        {isMerged && (
          <div className="absolute inset-0 flex items-center justify-center rounded-full bg-background/60 backdrop-blur-sm">
            <span className="text-[10px] font-medium text-muted-foreground">
              merged
            </span>
          </div>
        )}

        {/* ホバーツールチップ */}
        <div
          className={cn(
            "absolute -bottom-12 left-1/2 z-50 -translate-x-1/2",
            "rounded-lg border border-border/50 bg-card/95 px-3 py-1.5 backdrop-blur-md",
            "opacity-0 shadow-xl transition-all duration-300",
            "group-hover:opacity-100 group-hover:-translate-y-1",
          )}
          style={{
            boxShadow: `0 4px 20px hsl(220 30% 0% / 0.5), 0 0 20px ${colorStyle.glow}`,
          }}
        >
          <p className="whitespace-nowrap text-xs font-medium text-foreground">
            {displayName}
          </p>
          <p className="text-[10px] text-muted-foreground">
            {branch.type === "local" ? "Local" : "Remote"}
            {hasWorktree && " • Active"}
          </p>
        </div>
      </div>

      {/* 出力ハンドル - アクソン接続点 */}
      <Handle
        type="source"
        position={Position.Bottom}
        className="!h-3 !w-3 !rounded-full !border-2 !bg-transparent transition-all duration-300"
        style={{
          borderColor: colorStyle.primary,
          boxShadow: `0 0 8px ${colorStyle.glow}`,
        }}
      />
    </>
  );
});
