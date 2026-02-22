import type { StructuredError } from "./errorBus";

export interface BugReportData {
  title: string;
  stepsToReproduce: string;
  expectedResult: string;
  actualResult: string;
  systemInfo?: string;
  logs?: string;
  screenCapture?: string;
  screenshotPath?: string;
  error?: StructuredError;
  gwtVersion?: string;
  platform?: string;
}

export interface FeatureRequestData {
  title: string;
  description: string;
  useCase: string;
  expectedBenefit: string;
  gwtVersion?: string;
  platform?: string;
}

export function generateBugReportBody(data: BugReportData): string {
  const sections: string[] = [];
  sections.push("## Bug Report");
  sections.push("");
  sections.push("### Steps to Reproduce");
  sections.push(data.stepsToReproduce || "_No steps provided_");
  sections.push("");
  sections.push("### Expected Result");
  sections.push(data.expectedResult || "_Not specified_");
  sections.push("");
  sections.push("### Actual Result");
  sections.push(data.actualResult || "_Not specified_");

  if (data.error) {
    sections.push("");
    sections.push("---");
    sections.push("");
    sections.push("### Error Details");
    sections.push(`- **Code**: ${data.error.code}`);
    sections.push(`- **Severity**: ${data.error.severity}`);
    sections.push(`- **Command**: ${data.error.command}`);
    sections.push(`- **Category**: ${data.error.category}`);
    sections.push(`- **Timestamp**: ${data.error.timestamp}`);
    if (data.error.suggestions.length > 0) {
      sections.push(`- **Suggestions**: ${data.error.suggestions.join(", ")}`);
    }
  }

  sections.push("");
  sections.push("---");
  sections.push("");
  sections.push("### Diagnostic Information");

  if (data.systemInfo) {
    sections.push("");
    sections.push("#### System");
    sections.push(data.systemInfo);
  }

  if (data.gwtVersion || data.platform) {
    sections.push("");
    sections.push(`- **gwt Version**: ${data.gwtVersion ?? "unknown"}`);
    sections.push(`- **Platform**: ${data.platform ?? "unknown"}`);
  }

  if (data.screenCapture) {
    sections.push("");
    sections.push("#### Screen Capture");
    sections.push("```");
    sections.push(data.screenCapture);
    sections.push("```");
  }

  if (data.screenshotPath) {
    sections.push("");
    sections.push("#### Screenshot");
    sections.push(`Local path: \`${data.screenshotPath}\``);
  }

  if (data.logs) {
    sections.push("");
    sections.push("#### Application Logs");
    sections.push("```");
    sections.push(data.logs);
    sections.push("```");
  }

  return sections.join("\n");
}

export function generateFeatureRequestBody(data: FeatureRequestData): string {
  const sections: string[] = [];
  sections.push("## Feature Request");
  sections.push("");
  sections.push("### Description");
  sections.push(data.description || "_No description provided_");
  sections.push("");
  sections.push("### Use Case");
  sections.push(data.useCase || "_Not specified_");
  sections.push("");
  sections.push("### Expected Benefit");
  sections.push(data.expectedBenefit || "_Not specified_");
  sections.push("");
  sections.push("---");
  sections.push("");
  sections.push("### Context");
  sections.push(`- **gwt Version**: ${data.gwtVersion ?? "unknown"}`);
  sections.push(`- **Platform**: ${data.platform ?? "unknown"}`);
  return sections.join("\n");
}
