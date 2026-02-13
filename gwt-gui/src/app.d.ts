// See https://svelte.dev/docs/kit/types#app
declare module "svelte/elements" {
  interface HTMLTextareaAttributes {
    // Safari supports autocorrect on textarea, but upstream typings do not include it.
    autocorrect?: "on" | "off" | "" | undefined | null;
  }
}

declare global {
  namespace App {}
}
export {};
