import type { Page } from "@playwright/test";

// Issue #7880 (design item 137): Works has no rail entry. A person reaches
// the terminal host from the Issue detail or the Agents screen; specs that
// exercise the host itself open it through the same openSurface call.
export async function openWorks(page: Page): Promise<void> {
  // Interval polling: native hidden pages do not run animation frames.
  await page.waitForFunction(() => Boolean((window as any).__gwtSurfaceTestApi), undefined, { polling: 100 });
  await page.evaluate(() => (window as any).__gwtSurfaceTestApi.open("works"));
}
