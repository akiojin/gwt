/**
 * シナプスエッジコンポーネント
 *
 * 神経細胞の軸索（アクソン）をイメージした脈動するエッジ
 * 神経伝達物質の放出と伝播を表現するアニメーション
 */

import React, { memo, useMemo } from "react";
import {
  BaseEdge,
  EdgeLabelRenderer,
  getBezierPath,
  type EdgeProps,
} from "@xyflow/react";
import type { GraphEdge } from "./graphUtils";

type SynapticEdgeProps = EdgeProps<GraphEdge>;

export const SynapticEdge = memo(function SynapticEdge({
  id,
  sourceX,
  sourceY,
  targetX,
  targetY,
  sourcePosition,
  targetPosition,
  style = {},
  markerEnd,
}: SynapticEdgeProps) {
  const [edgePath, labelX, labelY] = getBezierPath({
    sourceX,
    sourceY,
    sourcePosition,
    targetX,
    targetY,
    targetPosition,
    curvature: 0.25,
  });

  // エッジの長さに基づいてパーティクル数を調整
  const edgeLength = useMemo(() => {
    const dx = targetX - sourceX;
    const dy = targetY - sourceY;
    return Math.sqrt(dx * dx + dy * dy);
  }, [sourceX, sourceY, targetX, targetY]);

  const particleCount = Math.max(2, Math.min(4, Math.floor(edgeLength / 100)));

  return (
    <>
      {/* 外側グロー - 深い影 */}
      <BaseEdge
        id={`${id}-outer-glow`}
        path={edgePath}
        style={{
          ...style,
          strokeWidth: 12,
          stroke: "hsl(185 100% 65% / 0.08)",
          filter: "blur(8px)",
        }}
      />

      {/* 中間グロー */}
      <BaseEdge
        id={`${id}-mid-glow`}
        path={edgePath}
        style={{
          ...style,
          strokeWidth: 6,
          stroke: "hsl(185 100% 65% / 0.15)",
          filter: "blur(4px)",
        }}
      />

      {/* メインエッジ - 軸索 */}
      <BaseEdge
        id={id}
        path={edgePath}
        {...(markerEnd ? { markerEnd } : {})}
        style={{
          ...style,
          strokeWidth: 2,
          stroke: "hsl(185 100% 65% / 0.5)",
          strokeLinecap: "round",
        }}
      />

      {/* 内側の明るいコア */}
      <BaseEdge
        id={`${id}-core`}
        path={edgePath}
        style={{
          ...style,
          strokeWidth: 1,
          stroke: "hsl(185 100% 75% / 0.6)",
          strokeLinecap: "round",
        }}
      />

      {/* SVGアニメーション定義 */}
      <EdgeLabelRenderer>
        <svg
          style={{
            position: "absolute",
            left: 0,
            top: 0,
            width: "100%",
            height: "100%",
            pointerEvents: "none",
            overflow: "visible",
          }}
        >
          <defs>
            {/* 神経伝達物質パルスグラデーション */}
            <linearGradient
              id={`vesicle-pulse-${id}`}
              x1="0%"
              y1="0%"
              x2="100%"
              y2="0%"
            >
              <stop offset="0%" stopColor="transparent">
                <animate
                  attributeName="offset"
                  values="-0.3;1"
                  dur="2.5s"
                  repeatCount="indefinite"
                />
              </stop>
              <stop offset="5%" stopColor="hsl(185 100% 70% / 0.3)">
                <animate
                  attributeName="offset"
                  values="-0.25;1.05"
                  dur="2.5s"
                  repeatCount="indefinite"
                />
              </stop>
              <stop offset="10%" stopColor="hsl(185 100% 80%)">
                <animate
                  attributeName="offset"
                  values="-0.2;1.1"
                  dur="2.5s"
                  repeatCount="indefinite"
                />
              </stop>
              <stop offset="15%" stopColor="hsl(185 100% 70% / 0.3)">
                <animate
                  attributeName="offset"
                  values="-0.15;1.15"
                  dur="2.5s"
                  repeatCount="indefinite"
                />
              </stop>
              <stop offset="20%" stopColor="transparent">
                <animate
                  attributeName="offset"
                  values="-0.1;1.2"
                  dur="2.5s"
                  repeatCount="indefinite"
                />
              </stop>
            </linearGradient>

            {/* 二次パルス（シナプス間隙） */}
            <linearGradient
              id={`synapse-pulse-${id}`}
              x1="0%"
              y1="0%"
              x2="100%"
              y2="0%"
            >
              <stop offset="0%" stopColor="transparent">
                <animate
                  attributeName="offset"
                  values="-0.4;1"
                  dur="3s"
                  begin="0.8s"
                  repeatCount="indefinite"
                />
              </stop>
              <stop offset="8%" stopColor="hsl(310 85% 65% / 0.6)">
                <animate
                  attributeName="offset"
                  values="-0.32;1.08"
                  dur="3s"
                  begin="0.8s"
                  repeatCount="indefinite"
                />
              </stop>
              <stop offset="16%" stopColor="transparent">
                <animate
                  attributeName="offset"
                  values="-0.24;1.16"
                  dur="3s"
                  begin="0.8s"
                  repeatCount="indefinite"
                />
              </stop>
            </linearGradient>

            {/* グロー効果フィルター */}
            <filter
              id={`glow-${id}`}
              x="-50%"
              y="-50%"
              width="200%"
              height="200%"
            >
              <feGaussianBlur stdDeviation="2" result="coloredBlur" />
              <feMerge>
                <feMergeNode in="coloredBlur" />
                <feMergeNode in="SourceGraphic" />
              </feMerge>
            </filter>
          </defs>

          {/* メインパルス */}
          <path
            d={edgePath}
            fill="none"
            stroke={`url(#vesicle-pulse-${id})`}
            strokeWidth={4}
            strokeLinecap="round"
            filter={`url(#glow-${id})`}
            style={{ mixBlendMode: "screen" }}
          />

          {/* セカンダリパルス（マゼンタ） */}
          <path
            d={edgePath}
            fill="none"
            stroke={`url(#synapse-pulse-${id})`}
            strokeWidth={3}
            strokeLinecap="round"
            style={{ mixBlendMode: "screen" }}
          />
        </svg>
      </EdgeLabelRenderer>

      {/* 流れるパーティクル（神経伝達物質） - SVG animateMotion使用 */}
      <EdgeLabelRenderer>
        <svg
          style={{
            position: "absolute",
            left: 0,
            top: 0,
            width: "100%",
            height: "100%",
            pointerEvents: "none",
            overflow: "visible",
          }}
        >
          {Array.from({ length: particleCount }).map((_, i) => (
            <circle
              key={i}
              r={3 - i * 0.3}
              fill="hsl(185 100% 80%)"
              opacity={0}
            >
              <animateMotion
                dur={`${2 + i * 0.3}s`}
                repeatCount="indefinite"
                path={edgePath}
                begin={`${i * 0.6}s`}
              />
              <animate
                attributeName="opacity"
                values="0;1;1;0"
                keyTimes="0;0.1;0.9;1"
                dur={`${2 + i * 0.3}s`}
                repeatCount="indefinite"
                begin={`${i * 0.6}s`}
              />
            </circle>
          ))}
        </svg>
      </EdgeLabelRenderer>

      {/* シナプス接合点グロー */}
      <EdgeLabelRenderer>
        <div
          className="absolute"
          style={{
            transform: `translate(-50%, -50%) translate(${labelX}px, ${labelY}px)`,
          }}
        >
          {/* 外側の拡散グロー */}
          <div
            className="absolute -inset-3 rounded-full opacity-30"
            style={{
              background:
                "radial-gradient(circle, hsl(185 100% 65%) 0%, transparent 70%)",
              animation: "synapse-spark 3s ease-in-out infinite",
            }}
          />
          {/* 中心核 */}
          <div
            className="h-2 w-2 rounded-full"
            style={{
              background:
                "radial-gradient(circle at 30% 30%, hsl(185 100% 80%), hsl(185 100% 60%))",
              boxShadow:
                "0 0 10px hsl(185 100% 65% / 0.8), 0 0 20px hsl(185 100% 65% / 0.4)",
              animation: "vesicle-release 2s ease-in-out infinite",
            }}
          />
        </div>
      </EdgeLabelRenderer>
    </>
  );
});
