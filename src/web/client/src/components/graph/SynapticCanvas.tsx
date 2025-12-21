/**
 * シナプティックキャンバス
 *
 * React Flowベースのブランチグラフ表示コンテナ
 * パン/ズーム/ミニマップを提供
 */

import React, { useCallback, useEffect, useState } from "react";
import {
  ReactFlow,
  Background,
  Controls,
  MiniMap,
  useNodesState,
  useEdgesState,
  type NodeMouseHandler,
  BackgroundVariant,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";

import type { Branch } from "../../../../../types/api.js";
import { BranchNode } from "./BranchNode";
import { ClusterNode } from "./ClusterNode";
import { SynapticEdge } from "./SynapticEdge";
import { branchesToGraph, type GraphNode, type GraphEdge } from "./graphUtils";

/** カスタムノードタイプ */
const nodeTypes = {
  branch: BranchNode,
  cluster: ClusterNode,
};

/** カスタムエッジタイプ */
const edgeTypes = {
  synaptic: SynapticEdge,
};

interface SynapticCanvasProps {
  branches: Branch[];
  onNodeClick?: (branch: Branch | null) => void;
  className?: string;
}

export function SynapticCanvas({
  branches,
  onNodeClick,
  className,
}: SynapticCanvasProps) {
  const [nodes, setNodes, onNodesChange] = useNodesState<GraphNode>([]);
  const [edges, setEdges, onEdgesChange] = useEdgesState<GraphEdge>([]);
  const [expandedClusters, setExpandedClusters] = useState<Set<string>>(
    new Set(),
  );
  const [isLayouting, setIsLayouting] = useState(false);

  // レイアウト計算
  useEffect(() => {
    if (branches.length === 0) return;

    setIsLayouting(true);

    // キャンバスサイズ（仮）
    const width = 800;
    const height = 600;

    branchesToGraph(branches, expandedClusters, width, height)
      .then(({ nodes: newNodes, edges: newEdges }) => {
        setNodes(newNodes);
        setEdges(newEdges);
      })
      .catch((error) => {
        console.error("Failed to calculate graph layout:", error);
      })
      .finally(() => {
        setIsLayouting(false);
      });
  }, [branches, expandedClusters, setNodes, setEdges]);

  // ノードクリックハンドラ
  const handleNodeClick: NodeMouseHandler<GraphNode> = useCallback(
    (event, node) => {
      if (node.data.isCluster) {
        // クラスタノード: 展開/折りたたみ
        setExpandedClusters((prev) => {
          const next = new Set(prev);
          if (next.has(node.id)) {
            next.delete(node.id);
          } else {
            next.add(node.id);
          }
          return next;
        });
      } else {
        // ブランチノード: 詳細パネル表示
        onNodeClick?.(node.data.branch ?? null);
      }
    },
    [onNodeClick],
  );

  // ミニマップのノードカラー
  const minimapNodeColor = useCallback((node: GraphNode) => {
    if (node.data.isCluster) {
      return "hsl(var(--muted-foreground))";
    }
    if (node.data.branch?.worktreePath) {
      return "hsl(var(--success))";
    }
    if (node.data.branch?.type === "local") {
      return "hsl(var(--local))";
    }
    return "hsl(var(--remote))";
  }, []);

  return (
    <div className={className} style={{ width: "100%", height: "100%" }}>
      <ReactFlow
        nodes={nodes}
        edges={edges}
        onNodesChange={onNodesChange}
        onEdgesChange={onEdgesChange}
        onNodeClick={handleNodeClick}
        nodeTypes={nodeTypes}
        edgeTypes={edgeTypes}
        fitView
        fitViewOptions={{
          padding: 0.2,
          maxZoom: 1.5,
        }}
        minZoom={0.1}
        maxZoom={2}
        attributionPosition="bottom-left"
        proOptions={{ hideAttribution: true }}
        className="bg-gradient-to-br from-background via-background to-muted/20"
      >
        {/* 背景グリッド */}
        <Background
          variant={BackgroundVariant.Dots}
          gap={20}
          size={1}
          color="hsl(var(--muted-foreground) / 0.15)"
        />

        {/* コントロール */}
        <Controls
          showInteractive={false}
          className="!bg-card !border !border-border !shadow-md"
        />

        {/* ミニマップ */}
        <MiniMap
          nodeColor={minimapNodeColor}
          maskColor="hsl(var(--background) / 0.8)"
          className="!bg-card !border !border-border !shadow-md"
        />
      </ReactFlow>

      {/* ローディングオーバーレイ */}
      {isLayouting && (
        <div className="absolute inset-0 flex items-center justify-center bg-background/50 backdrop-blur-sm">
          <div className="flex items-center gap-2 rounded-lg bg-card px-4 py-2 shadow-lg">
            <div className="h-4 w-4 animate-spin rounded-full border-2 border-primary border-t-transparent" />
            <span className="text-sm text-muted-foreground">
              Calculating layout...
            </span>
          </div>
        </div>
      )}
    </div>
  );
}
