import { useState, useCallback } from "react";
import { BatchMergeService } from "../../services/BatchMergeService.js";
import type {
  BatchMergeConfig,
  BatchMergeProgress,
  BatchMergeResult,
  BranchMergeStatus,
} from "../types.js";

/**
 * useBatchMerge hook - Manages batch merge state and execution
 * @see specs/SPEC-ee33ca26/plan.md - Service layer integration
 */
export function useBatchMerge() {
  const [isExecuting, setIsExecuting] = useState(false);
  const [progress, setProgress] = useState<BatchMergeProgress | null>(null);
  const [statuses, setStatuses] = useState<BranchMergeStatus[]>([]);
  const [result, setResult] = useState<BatchMergeResult | null>(null);
  const [error, setError] = useState<Error | null>(null);

  const service = new BatchMergeService();

  /**
   * Execute batch merge
   */
  const executeBatchMerge = useCallback(
    async (config: BatchMergeConfig) => {
      try {
        setIsExecuting(true);
        setProgress(null);
        setStatuses([]);
        setResult(null);
        setError(null);

        const mergeResult = await service.executeBatchMerge(
          config,
          (progressUpdate) => {
            setProgress(progressUpdate);

            // Update statuses as branches are processed
            // This is a simplified version; real implementation would track completed branches
          },
        );

        setResult(mergeResult);
        setStatuses(mergeResult.statuses);
        return mergeResult;
      } catch (err) {
        const error = err instanceof Error ? err : new Error(String(err));
        setError(error);
        throw error;
      } finally {
        setIsExecuting(false);
        setProgress(null);
      }
    },
    [service],
  );

  /**
   * Determine source branch automatically
   */
  const determineSourceBranch = useCallback(async () => {
    return await service.determineSourceBranch();
  }, [service]);

  /**
   * Get target branches
   */
  const getTargetBranches = useCallback(async () => {
    return await service.getTargetBranches();
  }, [service]);

  /**
   * Reset state
   */
  const reset = useCallback(() => {
    setIsExecuting(false);
    setProgress(null);
    setStatuses([]);
    setResult(null);
    setError(null);
  }, []);

  return {
    isExecuting,
    progress,
    statuses,
    result,
    error,
    executeBatchMerge,
    determineSourceBranch,
    getTargetBranches,
    reset,
  };
}
