// Hosts own imperative DOM (SPEC #6943): focus placement is an imperative
// operation, so surfaces decide *which* element should receive focus and
// this module performs the focus itself.
export function focusElement(element) {
  element?.focus();
}

export function focusFirstEnabledButton(element) {
  if (!element) return;
  const button = [...element.querySelectorAll("button")].find(candidate => !candidate.disabled);
  button?.focus();
}
