module.exports = {
  parserPreset: {
    name: "subject-only",
    parserOpts: {
      headerPattern: /^(.*)$/,
      headerCorrespondence: ["subject"],
    },
  },
  rules: {
    "subject-empty": [2, "never"],
    "subject-max-length": [2, "always", 100],
  },
};
