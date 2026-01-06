/** @jsxImportSource @opentui/solid */
import { render } from "@opentui/solid";
import type { CliRendererConfig } from "@opentui/core";
import { AppSolid, type AppSolidProps } from "./App.solid.js";

export { AppSolid, type AppSolidProps };

export async function renderSolidApp(
  props: AppSolidProps,
  config?: CliRendererConfig,
): Promise<void> {
  let resolveDone: (() => void) | null = null;
  const done = new Promise<void>((resolve) => {
    resolveDone = resolve;
  });
  const onDestroy = config?.onDestroy;

  await render(() => AppSolid(props), {
    ...config,
    onDestroy: () => {
      onDestroy?.();
      resolveDone?.();
      resolveDone = null;
    },
  });

  await done;
}
