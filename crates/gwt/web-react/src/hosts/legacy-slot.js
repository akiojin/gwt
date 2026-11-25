// Hosts own imperative DOM (SPEC #6943). A legacy slot lets a React surface
// reserve an element whose contents an unmigrated renderer still draws, so
// that renderer is reused instead of being implemented a second time.
export function fillLegacySlot(element, render) {
  if (element) render(element);
}
