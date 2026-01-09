const config = require("./.commitlintrc.json");

module.exports = {
  ...config,
  // Ignore merge commits (they don't follow Conventional Commits format)
  ignores: [(commit) => commit.startsWith("Merge ")],
};
