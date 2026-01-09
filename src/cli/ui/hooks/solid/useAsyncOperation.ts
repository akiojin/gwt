import { createMemo, createSignal, type Accessor } from "solid-js";
import type { AsyncState } from "../../core/types.js";

type AsyncOperation<T, Args extends unknown[]> = (...args: Args) => Promise<T>;

export interface UseAsyncOperationOptions<T> {
  initialState?: AsyncState<T>;
  onSuccess?: (data: T) => void;
  onError?: (error: Error) => void;
  onFinally?: () => void;
}

export interface UseAsyncOperationResult<T, Args extends unknown[]> {
  state: Accessor<AsyncState<T>>;
  setState: (state: AsyncState<T>) => void;
  isLoading: Accessor<boolean>;
  error: Accessor<Error | null>;
  data: Accessor<T | null>;
  run: (...args: Args) => Promise<T>;
  reset: () => void;
}

const toError = (err: unknown): Error =>
  err instanceof Error ? err : new Error(String(err));

export function useAsyncOperation<T, Args extends unknown[]>(
  operation: AsyncOperation<T, Args>,
  options: UseAsyncOperationOptions<T> = {},
): UseAsyncOperationResult<T, Args> {
  const [state, setStateInternal] = createSignal<AsyncState<T>>(
    options.initialState ?? { status: "idle" },
  );

  const isLoading = createMemo(() => state().status === "loading");
  const error = createMemo(() => {
    const current = state();
    return current.status === "error" ? current.error : null;
  });
  const data = createMemo(() => {
    const current = state();
    return current.status === "success" ? current.data : null;
  });

  const setState = (next: AsyncState<T>) => {
    setStateInternal(next);
  };

  const run = async (...args: Args) => {
    setState({ status: "loading" });
    try {
      const result = await operation(...args);
      setState({ status: "success", data: result });
      options.onSuccess?.(result);
      return result;
    } catch (err) {
      const errorValue = toError(err);
      setState({ status: "error", error: errorValue });
      options.onError?.(errorValue);
      throw errorValue;
    } finally {
      options.onFinally?.();
    }
  };

  const reset = () => {
    setState({ status: "idle" });
  };

  return {
    state,
    setState,
    isLoading,
    error,
    data,
    run,
    reset,
  };
}
