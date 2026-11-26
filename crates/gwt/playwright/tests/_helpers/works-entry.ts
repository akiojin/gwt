import { expect, type Page } from "@playwright/test";
import type { NativePage } from "./native-chromium";

// Issue #7880 (design item 137): Works has no rail entry. A person reaches
// the terminal host from the Issue detail or the Agents screen; specs that
// exercise the host itself open it through the same openSurface call.
export async function openWorks(page: Page | NativePage): Promise<void> {
  // Interval polling: native hidden pages do not run animation frames.
  await expect.poll(() => page.evaluate(() => Boolean((window as any).__gwtSurfaceTestApi)), {
    timeout: 30_000,
  }).toBe(true);
  await page.evaluate(() => (window as any).__gwtSurfaceTestApi.open("works"));
}
