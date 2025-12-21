/**
 * ブランチグラフ - シナプス風ビジュアライゼーション
 *
 * React Flow + D3-forceによるインタラクティブなブランチ関係図
 */

import React, { useState, useCallback } from "react";
import { Card, CardHeader, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import type { Branch } from "../../../../types/api.js";
import { SynapticCanvas, BranchDetailPanel } from "./graph";

interface BranchGraphProps {
  branches: Branch[];
}

export function BranchGraph({ branches }: BranchGraphProps) {
  const [selectedBranch, setSelectedBranch] = useState<Branch | null>(null);

  const handleNodeClick = useCallback((branch: Branch | null) => {
    setSelectedBranch(branch);
  }, []);

  const handlePanelClose = useCallback(() => {
    setSelectedBranch(null);
  }, []);

  if (!branches.length) {
    return (
      <Card className="border-dashed">
        <CardContent className="flex flex-col items-center justify-center py-12 text-center">
          <p className="text-muted-foreground">
            グラフ表示できるブランチがありません。
          </p>
          <p className="text-sm text-muted-foreground">
            fetch済みのブランチやWorktreeを追加すると関係図が表示されます。
          </p>
        </CardContent>
      </Card>
    );
  }

  return (
    <Card className="relative overflow-hidden">
      <CardHeader className="pb-4">
        <div className="flex flex-wrap items-start justify-between gap-4">
          <div>
            <p className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
              SYNAPTIC GRAPH
            </p>
            <h2 className="mt-1 text-lg font-semibold">ブランチネットワーク</h2>
            <p className="mt-1 text-sm text-muted-foreground">
              クリックでノードを展開・詳細表示。ドラッグで移動、スクロールでズーム。
            </p>
          </div>
          <div className="flex flex-wrap gap-2">
            <Badge variant="outline" className="flex items-center gap-1">
              <span className="h-2 w-2 rounded-full bg-muted-foreground/30" />
              Cluster
            </Badge>
            <Badge variant="local" className="flex items-center gap-1">
              <span className="h-2 w-2 rounded-full bg-local" />
              Local
            </Badge>
            <Badge variant="remote" className="flex items-center gap-1">
              <span className="h-2 w-2 rounded-full bg-remote" />
              Remote
            </Badge>
            <Badge variant="success" className="flex items-center gap-1">
              <span className="h-2 w-2 rounded-full bg-success" />
              Worktree
            </Badge>
          </div>
        </div>
      </CardHeader>

      <CardContent className="relative p-0">
        {/* キャンバスコンテナ */}
        <div className="relative h-[60vh] min-h-[360px] max-h-[640px] w-full">
          <SynapticCanvas
            branches={branches}
            onNodeClick={handleNodeClick}
            className="h-full w-full"
          />

          {/* 詳細パネル */}
          <BranchDetailPanel
            branch={selectedBranch}
            onClose={handlePanelClose}
          />
        </div>
      </CardContent>
    </Card>
  );
}
