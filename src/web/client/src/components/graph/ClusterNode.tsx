/**
 * クラスタノードコンポーネント
 *
 * ニューロン群（神経細胞集合体）をイメージした有機的ノード
 * 変形する膜と軌道を描く核で複数ブランチのグループを表現
 */

import React, { memo, useMemo } from "react";
import { Handle, Position, type NodeProps } from "@xyflow/react";
import { cn } from "@/lib/utils";
import type { GraphNode } from "./graphUtils";

type ClusterNodeProps = NodeProps<GraphNode>;

export const ClusterNode = memo(function ClusterNode({
  data,
  selected,
}: ClusterNodeProps) {
  const { clusterSize = 0, expanded } = data;

  // クラスタサイズに応じてノードサイズを調整
  const baseSize = 70;
  const size = Math.min(baseSize + clusterSize * 5, 120);

  // 内部の核の数（最大8個）
  const nucleiCount = Math.min(clusterSize, 8);

  // 核の配置角度を計算
  const nuclei = useMemo(() => {
    return Array.from({ length: nucleiCount }).map((_, i) => {
      const angle = (360 / nucleiCount) * i;
      const delay = i * 0.5;
      const orbitRadius = size * 0.28;
      return { angle, delay, orbitRadius };
    });
  }, [nucleiCount, size]);

  return (
    <>
      {/* 入力ハンドル */}
      <Handle
        type="target"
        position={Position.Top}
        className="!h-3 !w-3 !rounded-full !border-2 !bg-transparent transition-all duration-300"
        style={{
          borderColor: "hsl(185 100% 65%)",
          boxShadow: "0 0 8px hsl(185 100% 65% / 0.4)",
        }}
      />

      {/* クラスタ本体 */}
      <div
        className={cn(
          "group relative flex cursor-pointer items-center justify-center",
          "transition-all duration-500 ease-out",
          "animate-membrane-morph",
          selected &&
            "ring-2 ring-primary ring-offset-4 ring-offset-background",
        )}
        style={{
          width: size,
          height: size,
          background: expanded
            ? "radial-gradient(circle at 40% 40%, hsl(185 100% 65% / 0.15), hsl(310 85% 60% / 0.08) 50%, transparent 70%)"
            : "radial-gradient(circle at 40% 40%, hsl(220 30% 15% / 0.8), hsl(220 25% 10% / 0.6) 60%, transparent 80%)",
          border: expanded
            ? "2px solid hsl(185 100% 65% / 0.6)"
            : "2px dashed hsl(200 30% 30%)",
          boxShadow: expanded
            ? "0 0 30px hsl(185 100% 65% / 0.3), inset 0 0 30px hsl(185 100% 65% / 0.1)"
            : "0 0 20px hsl(220 30% 0% / 0.5), inset 0 0 20px hsl(220 30% 5% / 0.5)",
        }}
      >
        {/* 外殻グロー */}
        <div
          className={cn(
            "absolute -inset-3 rounded-full blur-lg transition-opacity duration-500",
            expanded ? "opacity-40" : "opacity-20",
          )}
          style={{
            backgroundColor: expanded
              ? "hsl(185 100% 65%)"
              : "hsl(200 30% 40%)",
            borderRadius: "inherit",
          }}
        />

        {/* 内部膜 */}
        <div
          className="absolute inset-3 rounded-full border border-dashed opacity-30"
          style={{
            borderColor: expanded ? "hsl(185 100% 65%)" : "hsl(200 30% 40%)",
            borderRadius: "inherit",
          }}
        />

        {/* 軌道を描く核 */}
        <div className="absolute inset-0 flex items-center justify-center">
          {nuclei.map((nucleus, i) => (
            <div
              key={i}
              className="absolute animate-nucleus-orbit"
              style={{
                animationDelay: `${-nucleus.delay}s`,
                animationDuration: `${8 + i * 0.5}s`,
              }}
            >
              <div
                className="h-2.5 w-2.5 rounded-full animate-vesicle-release"
                style={{
                  backgroundColor: expanded
                    ? `hsl(${185 + i * 15} 80% 60%)`
                    : `hsl(${200 + i * 10} 50% 50%)`,
                  boxShadow: expanded
                    ? `0 0 8px hsl(${185 + i * 15} 80% 60% / 0.8)`
                    : `0 0 4px hsl(${200 + i * 10} 50% 50% / 0.5)`,
                  animationDelay: `${nucleus.delay * 0.3}s`,
                  transform: `translateX(${nucleus.orbitRadius}px)`,
                }}
              />
            </div>
          ))}
        </div>

        {/* 中心核 - クラスタサイズ表示 */}
        <div
          className={cn(
            "relative z-10 flex items-center justify-center rounded-full",
            "border-2 backdrop-blur-sm transition-all duration-300",
          )}
          style={{
            width: size * 0.4,
            height: size * 0.4,
            backgroundColor: expanded
              ? "hsl(220 30% 8% / 0.9)"
              : "hsl(220 30% 10% / 0.95)",
            borderColor: expanded
              ? "hsl(185 100% 65% / 0.8)"
              : "hsl(200 30% 35%)",
            boxShadow: expanded
              ? "0 0 20px hsl(185 100% 65% / 0.4), inset 0 0 15px hsl(185 100% 65% / 0.1)"
              : "0 0 10px hsl(220 30% 0% / 0.5)",
          }}
        >
          <span
            className={cn(
              "text-sm font-bold transition-colors duration-300",
              expanded ? "text-primary" : "text-muted-foreground",
            )}
          >
            {clusterSize}
          </span>
        </div>

        {/* 展開/折りたたみインジケータ */}
        <div
          className={cn(
            "absolute -right-2 -top-2 flex h-6 w-6 items-center justify-center rounded-full",
            "border-2 text-xs font-bold transition-all duration-300",
          )}
          style={{
            backgroundColor: expanded
              ? "hsl(185 100% 65%)"
              : "hsl(220 30% 12%)",
            borderColor: expanded ? "hsl(185 100% 70%)" : "hsl(200 30% 25%)",
            color: expanded ? "hsl(220 30% 5%)" : "hsl(185 100% 65%)",
            boxShadow: expanded
              ? "0 0 15px hsl(185 100% 65% / 0.6)"
              : "0 0 8px hsl(220 30% 0% / 0.5)",
          }}
        >
          {expanded ? "−" : "+"}
        </div>

        {/* ホバーツールチップ */}
        <div
          className={cn(
            "absolute -bottom-14 left-1/2 z-50 -translate-x-1/2",
            "rounded-lg border border-border/50 bg-card/95 px-3 py-2 backdrop-blur-md",
            "opacity-0 shadow-xl transition-all duration-300",
            "group-hover:opacity-100 group-hover:-translate-y-1",
          )}
          style={{
            boxShadow:
              "0 4px 20px hsl(220 30% 0% / 0.5), 0 0 20px hsl(185 100% 65% / 0.2)",
          }}
        >
          <p className="whitespace-nowrap text-xs font-medium text-foreground">
            {clusterSize} branches
          </p>
          <p className="text-[10px] text-muted-foreground">
            Click to {expanded ? "collapse" : "expand"}
          </p>
        </div>
      </div>

      {/* 出力ハンドル */}
      <Handle
        type="source"
        position={Position.Bottom}
        className="!h-3 !w-3 !rounded-full !border-2 !bg-transparent transition-all duration-300"
        style={{
          borderColor: expanded ? "hsl(185 100% 65%)" : "hsl(200 30% 40%)",
          boxShadow: expanded
            ? "0 0 8px hsl(185 100% 65% / 0.4)"
            : "0 0 6px hsl(200 30% 40% / 0.3)",
        }}
      />
    </>
  );
});
