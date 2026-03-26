import { describe, expect, it } from "vitest";
import {
  createModalState,
  showReportDialogRuntime,
  openAboutDialogRuntime,
  openCleanupModalRuntime,
  type ModalState,
} from "./appModalStateRuntime";
import type { StructuredError } from "./errorBus";

describe("appModalStateRuntime", () => {
  describe("createModalState", () => {
    it("returns all dialogs closed with default values", () => {
      const state = createModalState();
      expect(state.about).toEqual({ open: false, initialTab: "general" });
      expect(state.cleanup).toEqual({ open: false, preselectedBranch: null });
      expect(state.reportDialog).toEqual({
        open: false,
        mode: "bug",
        prefillError: undefined,
      });
      expect(state.terminalDiagnostics).toEqual({
        open: false,
        loading: false,
        data: null,
        error: null,
      });
      expect(state.osEnvDebug).toEqual({
        open: false,
        data: null,
        loading: false,
        error: null,
      });
      expect(state.appError).toBeNull();
    });
  });

  describe("showReportDialogRuntime", () => {
    it("opens report dialog with bug mode", () => {
      const state = createModalState();
      showReportDialogRuntime(state, "bug");
      expect(state.reportDialog.open).toBe(true);
      expect(state.reportDialog.mode).toBe("bug");
      expect(state.reportDialog.prefillError).toBeUndefined();
    });

    it("opens report dialog with feature mode", () => {
      const state = createModalState();
      showReportDialogRuntime(state, "feature");
      expect(state.reportDialog.open).toBe(true);
      expect(state.reportDialog.mode).toBe("feature");
    });

    it("sets prefillError when provided", () => {
      const state = createModalState();
      const error: StructuredError = {
        severity: "error",
        code: "E001",
        message: "Something failed",
        command: "test-cmd",
        category: "test",
        suggestions: ["Try again"],
        timestamp: "2026-01-01T00:00:00Z",
      };
      showReportDialogRuntime(state, "bug", error);
      expect(state.reportDialog.prefillError).toBe(error);
    });
  });

  describe("openAboutDialogRuntime", () => {
    it("opens about dialog with default general tab", () => {
      const state = createModalState();
      openAboutDialogRuntime(state);
      expect(state.about.open).toBe(true);
      expect(state.about.initialTab).toBe("general");
    });

    it("opens about dialog with specified tab", () => {
      const state = createModalState();
      openAboutDialogRuntime(state, "system");
      expect(state.about.open).toBe(true);
      expect(state.about.initialTab).toBe("system");
    });

    it("opens about dialog with statistics tab", () => {
      const state = createModalState();
      openAboutDialogRuntime(state, "statistics");
      expect(state.about.open).toBe(true);
      expect(state.about.initialTab).toBe("statistics");
    });
  });

  describe("openCleanupModalRuntime", () => {
    it("opens cleanup modal with no preselected branch", () => {
      const state = createModalState();
      openCleanupModalRuntime(state);
      expect(state.cleanup.open).toBe(true);
      expect(state.cleanup.preselectedBranch).toBeNull();
    });

    it("opens cleanup modal with preselected branch", () => {
      const state = createModalState();
      openCleanupModalRuntime(state, "feature/test");
      expect(state.cleanup.open).toBe(true);
      expect(state.cleanup.preselectedBranch).toBe("feature/test");
    });

    it("sets preselectedBranch to null when null is passed", () => {
      const state = createModalState();
      openCleanupModalRuntime(state, null);
      expect(state.cleanup.open).toBe(true);
      expect(state.cleanup.preselectedBranch).toBeNull();
    });
  });

  describe("state independence", () => {
    it("creates independent state instances", () => {
      const state1 = createModalState();
      const state2 = createModalState();
      openAboutDialogRuntime(state1, "system");
      expect(state1.about.open).toBe(true);
      expect(state2.about.open).toBe(false);
    });

    it("opening one dialog does not affect others", () => {
      const state = createModalState();
      showReportDialogRuntime(state, "bug");
      expect(state.reportDialog.open).toBe(true);
      expect(state.about.open).toBe(false);
      expect(state.cleanup.open).toBe(false);
    });
  });
});
