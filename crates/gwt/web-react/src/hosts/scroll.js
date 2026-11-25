// Hosts own imperative DOM (SPEC #6943): scrolling is an imperative
// operation, so surfaces decide *where* a scroll container should be and this
// module performs the scroll itself.
export function scrollToEnd(element) {
  if (element) element.scrollTop = element.scrollHeight;
}

export function scrollIntoCenter(element) {
  element?.scrollIntoView?.({ block: "center" });
}


export function restoreScrollAnchor(container, element, top) {
  container.scrollTop += element.getBoundingClientRect().top - top;
}
