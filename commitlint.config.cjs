module.exports = {
  extends: ["@commitlint/config-conventional"],
  rules: {
    "subject-empty": [2, "never"],
    "subject-max-length": [2, "always", 100],
    "header-max-length": [2, "always", 100],
    "subject-case": [0],
    "body-max-line-length": [2, "always", 100],
    "type-enum": [
      2,
      "always",
      [
        "feat",
        "fix",
        "docs",
        "style",
        "refactor",
        "perf",
        "test",
        "build",
        "ci",
        "chore",
        "revert",
      ],
    ],
  },
  // Ignore commits that don't follow Conventional Commits format
  ignores: [
    (commit) => {
      const firstLine = commit.split("\n")[0].trim();
      // Merge commits
      if (/^merge[:\s]/i.test(firstLine)) return true;
      // Branch-name-style commits (historical)
      if (/^(bugfix|feature|hotfix|release)\//.test(firstLine)) return true;
      // Historical commits without conventional prefix (Fix/Stabilize pattern)
      if (/^(Fix|Stabilize|Update|Add|Remove|Refactor|Clean)\s/.test(firstLine))
        return true;
      return false;
    },
  ],
};
