import type { StructuredError } from "$lib/errorBus";
import { maskSensitiveData } from "$lib/privacyMask";
import {
  generateBugReportBody,
  generateFeatureRequestBody,
  type BugReportData,
  type FeatureRequestData,
} from "$lib/issueTemplate";

export interface ReportTarget {
  owner: string;
  repo: string;
  display: string;
}

export function buildScreenCaptureTextRuntime(args: {
  includeScreenCapture: boolean;
  screenCaptureText: string;
  terminalCaptureDone: boolean;
  terminalCaptureText: string;
}): string | undefined {
  const parts: string[] = [];
  if (args.includeScreenCapture && args.screenCaptureText) {
    parts.push(args.screenCaptureText);
  }
  if (args.terminalCaptureDone && args.terminalCaptureText) {
    parts.push(args.terminalCaptureText);
  }
  return parts.length > 0 ? parts.join("\n\n") : undefined;
}

export function generateReportBodyRuntime(args: {
  activeTab: "bug" | "feature";
  bugTitle: string;
  stepsToReproduce: string;
  expectedResult: string;
  actualResult: string;
  includeSystemInfo: boolean;
  systemInfoText: string;
  includeLogs: boolean;
  logsText: string;
  screenCaptureText?: string;
  prefillError?: StructuredError;
  featureTitle: string;
  featureDescription: string;
  useCase: string;
  expectedBenefit: string;
}): string {
  if (args.activeTab === "bug") {
    const data: BugReportData = {
      title: args.bugTitle,
      stepsToReproduce: args.stepsToReproduce,
      expectedResult: args.expectedResult,
      actualResult: args.actualResult,
      systemInfo: args.includeSystemInfo ? args.systemInfoText : undefined,
      logs: args.includeLogs ? args.logsText : undefined,
      screenCapture: args.screenCaptureText,
      error: args.prefillError,
    };
    return maskSensitiveData(generateBugReportBody(data));
  }

  const data: FeatureRequestData = {
    title: args.featureTitle,
    description: args.featureDescription,
    useCase: args.useCase,
    expectedBenefit: args.expectedBenefit,
  };
  return maskSensitiveData(generateFeatureRequestBody(data));
}

export function buildBrowserIssueUrlRuntime(args: {
  target: ReportTarget;
  activeTab: "bug" | "feature";
  title: string;
}): string {
  const labels = args.activeTab === "bug" ? "bug" : "enhancement";
  return `https://github.com/${args.target.owner}/${args.target.repo}/issues/new?title=${encodeURIComponent(args.title)}&labels=${encodeURIComponent(labels)}`;
}

export function normalizeDetectedTargetsRuntime(
  detected: ReportTarget,
  defaultTarget: ReportTarget,
): ReportTarget[] {
  if (detected.display === defaultTarget.display) {
    return [defaultTarget];
  }
  return [defaultTarget, detected];
}
