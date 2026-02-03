const resolveLocal = (specifier) => {
  try {
    return require.resolve(specifier, { paths: [__dirname] });
  } catch {
    return null;
  }
};

const hasConventionalConfig = Boolean(
  resolveLocal("@commitlint/config-conventional"),
);
const hasConventionalParser = Boolean(
  resolveLocal("conventional-changelog-conventionalcommits"),
);

const severity = {
  off: 0,
  warn: 1,
  error: 2,
};

const conventionalTypeEnum = [
  "build",
  "chore",
  "ci",
  "docs",
  "feat",
  "fix",
  "perf",
  "refactor",
  "revert",
  "style",
  "test",
];

const customTypeEnum = [
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
];

const conventionalRules = {
  "body-leading-blank": [severity.warn, "always"],
  "body-max-line-length": [severity.error, "always", 100],
  "footer-leading-blank": [severity.warn, "always"],
  "footer-max-line-length": [severity.error, "always", 100],
  "header-max-length": [severity.error, "always", 100],
  "header-trim": [severity.error, "always"],
  "subject-case": [
    severity.error,
    "never",
    ["sentence-case", "start-case", "pascal-case", "upper-case"],
  ],
  "subject-empty": [severity.error, "never"],
  "subject-full-stop": [severity.error, "never", "."],
  "type-case": [severity.error, "always", "lower-case"],
  "type-empty": [severity.error, "never"],
  "type-enum": [severity.error, "always", conventionalTypeEnum],
};

const customRules = {
  "subject-empty": [2, "never"],
  "subject-max-length": [2, "always", 100],
  "header-max-length": [2, "always", 100],
  "subject-case": [0],
  "body-max-line-length": [2, "always", 100],
  "type-enum": [2, "always", customTypeEnum],
};

const rules = hasConventionalConfig
  ? customRules
  : { ...conventionalRules, ...customRules };

module.exports = {
  ...(hasConventionalConfig
    ? { extends: ["@commitlint/config-conventional"] }
    : hasConventionalParser
      ? { parserPreset: "conventional-changelog-conventionalcommits" }
      : {}),
  rules,
  // Ignore commits that don't follow Conventional Commits format
  ignores: [
    (commit) => {
      const firstLine = commit.split("\n")[0].trim();
      // Merge commits
      if (/^merge[:\s]/i.test(firstLine)) return true;
      // Branch-name-style commits (historical)
      if (/^(bugfix|feature|hotfix|release)\//.test(firstLine)) return true;
      // Historical commits without conventional prefix (Fix/Stabilize pattern)
      if (
        /^(Fix|Stabilize|Update|Add|Remove|Refactor|Clean|Format|Resolve)\s/.test(
          firstLine,
        )
      )
        return true;
      return false;
    },
  ],
};
