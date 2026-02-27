import { describe, it, expect, beforeEach } from "vitest";
import {
  registerTerminalInputTarget,
  getFocusedTerminalPaneId,
  clearTerminalInputTargetsForTests,
} from "./inputTargetRegistry";

function createEl(tag = "div"): HTMLElement {
  return document.createElement(tag);
}

describe("inputTargetRegistry", () => {
  beforeEach(() => {
    clearTerminalInputTargetsForTests();
  });

  describe("registerTerminalInputTarget", () => {
    it("registers an element and returns a cleanup function", () => {
      const root = createEl();
      const cleanup = registerTerminalInputTarget("pane-1", root);
      expect(typeof cleanup).toBe("function");
    });

    it("cleanup removes the registration", () => {
      const root = createEl();
      document.body.appendChild(root);
      const inner = createEl("input");
      root.appendChild(inner);
      inner.focus();

      const cleanup = registerTerminalInputTarget("pane-1", root);
      expect(getFocusedTerminalPaneId(document)).toBe("pane-1");

      cleanup();
      expect(getFocusedTerminalPaneId(document)).toBeNull();

      document.body.removeChild(root);
    });

    it("does not remove registration if element was replaced", () => {
      const root1 = createEl();
      const root2 = createEl();
      document.body.appendChild(root2);
      const inner2 = createEl("input");
      root2.appendChild(inner2);

      const cleanup1 = registerTerminalInputTarget("pane-1", root1);
      registerTerminalInputTarget("pane-1", root2);

      // cleanup1 should NOT remove root2's registration because rootEl changed
      cleanup1();

      inner2.focus();
      expect(getFocusedTerminalPaneId(document)).toBe("pane-1");

      document.body.removeChild(root2);
    });
  });

  describe("getFocusedTerminalPaneId", () => {
    it("returns null when no targets registered", () => {
      expect(getFocusedTerminalPaneId(document)).toBeNull();
    });

    it("returns null when active element is not inside any target", () => {
      const root = createEl();
      registerTerminalInputTarget("pane-1", root);
      // document.body is the active element by default in JSDOM
      expect(getFocusedTerminalPaneId(document)).toBeNull();
    });

    it("returns paneId when active element is inside a registered target", () => {
      const root = createEl();
      document.body.appendChild(root);
      const inner = createEl("input");
      root.appendChild(inner);
      inner.focus();

      registerTerminalInputTarget("pane-2", root);
      expect(getFocusedTerminalPaneId(document)).toBe("pane-2");

      document.body.removeChild(root);
    });

    it("returns correct paneId with multiple targets", () => {
      const root1 = createEl();
      const root2 = createEl();
      document.body.appendChild(root1);
      document.body.appendChild(root2);

      const inner2 = createEl("input");
      root2.appendChild(inner2);

      registerTerminalInputTarget("pane-a", root1);
      registerTerminalInputTarget("pane-b", root2);

      inner2.focus();
      expect(getFocusedTerminalPaneId(document)).toBe("pane-b");

      document.body.removeChild(root1);
      document.body.removeChild(root2);
    });
  });

  describe("clearTerminalInputTargetsForTests", () => {
    it("clears all registrations", () => {
      const root = createEl();
      document.body.appendChild(root);
      const inner = createEl("input");
      root.appendChild(inner);
      inner.focus();

      registerTerminalInputTarget("pane-x", root);
      expect(getFocusedTerminalPaneId(document)).toBe("pane-x");

      clearTerminalInputTargetsForTests();
      expect(getFocusedTerminalPaneId(document)).toBeNull();

      document.body.removeChild(root);
    });
  });
});
