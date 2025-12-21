/**
 * Branch[] → React Flow Node/Edge 変換ユーティリティ
 *
 * D3-forceレイアウトと組み合わせてシナプス風配置を実現
 */

import {
  forceSimulation,
  forceLink,
  forceManyBody,
  forceCenter,
  forceCollide,
  type Simulation,
  type SimulationNodeDatum,
  type SimulationLinkDatum,
} from "d3-force";
import type { Node, Edge } from "@xyflow/react";
import type { Branch } from "../../../../../types/api.js";

/** グラフノードの拡張型 */
export interface GraphNode extends Node {
  data: {
    branch: Branch | undefined;
    isCluster: boolean;
    clusterSize: number;
    expanded: boolean;
  };
}

/** グラフエッジの型 */
export interface GraphEdge extends Edge {
  data?: {
    strength: number;
  };
}

/** クラスタ情報 */
export interface Cluster {
  id: string;
  baseBranch: string;
  branches: Branch[];
  isExpanded: boolean;
}

/** D3シミュレーション用ノード */
interface SimNode extends SimulationNodeDatum {
  id: string;
  branch?: Branch;
  isCluster: boolean;
  clusterSize?: number;
}

/** D3シミュレーション用リンク */
interface SimLink extends SimulationLinkDatum<SimNode> {
  source: string | SimNode;
  target: string | SimNode;
}

/**
 * ブランチをクラスタにグループ化
 */
export function clusterBranches(branches: Branch[]): Cluster[] {
  const clusterMap = new Map<string, Cluster>();

  // ベースブランチごとにグループ化
  for (const branch of branches) {
    const baseKey = branch.baseBranch ?? "__root__";

    if (!clusterMap.has(baseKey)) {
      clusterMap.set(baseKey, {
        id: `cluster-${baseKey}`,
        baseBranch: baseKey,
        branches: [],
        isExpanded: false,
      });
    }

    clusterMap.get(baseKey)?.branches.push(branch);
  }

  return Array.from(clusterMap.values()).sort((a, b) => {
    // ルートクラスタを先頭に
    if (a.baseBranch === "__root__") return -1;
    if (b.baseBranch === "__root__") return 1;
    // ブランチ数が多い順
    return b.branches.length - a.branches.length;
  });
}

/**
 * D3-forceシミュレーションを実行してノード位置を計算
 */
export function calculateLayout(
  nodes: SimNode[],
  links: SimLink[],
  width: number,
  height: number,
): Promise<SimNode[]> {
  return new Promise((resolve) => {
    const simulation: Simulation<SimNode, SimLink> = forceSimulation(nodes)
      .force(
        "link",
        forceLink<SimNode, SimLink>(links)
          .id((d) => d.id)
          .distance(120)
          .strength(0.8),
      )
      .force("charge", forceManyBody().strength(-400))
      .force("center", forceCenter(width / 2, height / 2))
      .force("collide", forceCollide().radius(60).strength(0.7));

    // 100ティック分シミュレーションを進める
    for (let i = 0; i < 100; i++) {
      simulation.tick();
    }

    simulation.stop();
    resolve(nodes);
  });
}

/**
 * Branch[] → GraphNode[] / GraphEdge[] 変換
 */
export async function branchesToGraph(
  branches: Branch[],
  expandedClusters: Set<string>,
  canvasWidth: number,
  canvasHeight: number,
): Promise<{ nodes: GraphNode[]; edges: GraphEdge[] }> {
  const clusters = clusterBranches(branches);
  const _branchMap = new Map(branches.map((b) => [b.name, b]));

  const simNodes: SimNode[] = [];
  const simLinks: SimLink[] = [];

  // ブランチ名からノードIDへのマッピング（クラスタ化されている場合はクラスタIDを返す）
  const branchToNodeId = new Map<string, string>();

  // 最初にノードIDマッピングを構築
  for (const cluster of clusters) {
    const isExpanded = expandedClusters.has(cluster.id);

    if (cluster.branches.length === 1 || isExpanded) {
      // 個別ノード: ブランチ名がそのままノードID
      for (const branch of cluster.branches) {
        branchToNodeId.set(branch.name, branch.name);
      }
    } else {
      // クラスタノード: 全ブランチがクラスタIDにマッピング
      for (const branch of cluster.branches) {
        branchToNodeId.set(branch.name, cluster.id);
      }
    }
  }

  // クラスタとノードを生成
  for (const cluster of clusters) {
    const isExpanded = expandedClusters.has(cluster.id);

    if (cluster.branches.length === 1 || isExpanded) {
      // 単一ブランチまたは展開済み: 個別ノードとして追加
      for (const branch of cluster.branches) {
        simNodes.push({
          id: branch.name,
          branch,
          isCluster: false,
        });

        // 親ブランチへのリンク（親がノードとして存在する場合のみ）
        if (branch.baseBranch) {
          const parentNodeId = branchToNodeId.get(branch.baseBranch);
          if (parentNodeId && parentNodeId !== branch.name) {
            simLinks.push({
              source: parentNodeId,
              target: branch.name,
            });
          }
        }
      }
    } else {
      // 折りたたみ状態: クラスタノードとして追加
      simNodes.push({
        id: cluster.id,
        isCluster: true,
        clusterSize: cluster.branches.length,
      });

      // クラスタから親ブランチへのリンク
      if (cluster.baseBranch !== "__root__") {
        const parentNodeId = branchToNodeId.get(cluster.baseBranch);
        if (parentNodeId && parentNodeId !== cluster.id) {
          simLinks.push({
            source: parentNodeId,
            target: cluster.id,
          });
        }
      }
    }
  }

  // D3-forceでレイアウト計算
  const layoutNodes = await calculateLayout(
    simNodes,
    simLinks,
    canvasWidth,
    canvasHeight,
  );

  // React Flow形式に変換
  const graphNodes: GraphNode[] = layoutNodes.map((node) => ({
    id: node.id,
    type: node.isCluster ? "cluster" : "branch",
    position: { x: node.x ?? 0, y: node.y ?? 0 },
    data: {
      branch: node.branch,
      isCluster: node.isCluster,
      clusterSize: node.clusterSize ?? 0,
      expanded: expandedClusters.has(node.id),
    },
  }));

  const graphEdges: GraphEdge[] = simLinks.map((link, idx) => {
    const sourceId =
      typeof link.source === "string" ? link.source : link.source.id;
    const targetId =
      typeof link.target === "string" ? link.target : link.target.id;

    return {
      id: `edge-${idx}`,
      source: sourceId,
      target: targetId,
      type: "synaptic",
      animated: true,
      data: { strength: 1 },
    };
  });

  return { nodes: graphNodes, edges: graphEdges };
}

/**
 * ノードの色を決定
 */
export function getNodeColor(branch: Branch): string {
  if (branch.worktreePath) {
    return "hsl(var(--success))";
  }
  if (branch.type === "local") {
    return "hsl(var(--local))";
  }
  return "hsl(var(--remote))";
}

/**
 * ノードサイズを決定（divergenceに基づく）
 */
export function getNodeSize(branch: Branch): number {
  const base = 40;
  if (!branch.divergence) return base;

  const activity = branch.divergence.ahead + branch.divergence.behind;
  // 活発なブランチは大きく表示
  return Math.min(base + activity * 2, 80);
}
