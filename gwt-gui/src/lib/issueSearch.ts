export interface IssueSearchTarget {
  number: number;
  title: string;
  labels?: string[];
  isSpec?: boolean;
}

function normalizeIssueNumberToken(token: string): string | null {
  const stripped = token.startsWith("#") ? token.slice(1) : token;
  return /^\d+$/.test(stripped) ? stripped : null;
}

function tokenizeIssueSearchQuery(query: string): string[] {
  return query
    .trim()
    .split(/\s+/)
    .filter((token) => token.length > 0);
}

export function issueMatchesSearchQuery(
  issue: IssueSearchTarget,
  query: string,
): boolean {
  const tokens = tokenizeIssueSearchQuery(query);
  if (tokens.length === 0) return true;

  const issueNumber = String(issue.number);
  const titleLower = issue.title.toLowerCase();
  const labelSet = new Set(
    (issue.labels ?? []).map((label) => label.toLowerCase()),
  );
  const isSpec = issue.isSpec === true || labelSet.has("gwt-spec");

  return tokens.every((token) => {
    const numberToken = normalizeIssueNumberToken(token);
    if (numberToken !== null) {
      return issueNumber.includes(numberToken);
    }
    const normalized = token.toLowerCase();
    if (normalized === "spec" || normalized === "specs") {
      return isSpec;
    }
    if (titleLower.includes(normalized)) {
      return true;
    }
    return Array.from(labelSet).some((label) => label.includes(normalized));
  });
}
