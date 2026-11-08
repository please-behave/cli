// Share revert parsing between version calculation and release notes, including scopes.
const parserOpts = {
  revertPattern: /^revert(?:\([^)]+\))?!?:\s"?([\s\S]+?)"?\s*This reverts commit (\w*)\./i,
  revertCorrespondence: ["header", "hash"],
};

export default {
  branches: ["main"],
  tagFormat: "v${version}",
  plugins: [
    ["./scripts/release-analyzer.mjs", {
      developmentSeries: true,
      preset: "conventionalcommits",
      parserOpts,
      releaseRules: [
        { breaking: true, release: "major" },
        { type: "feat", release: "minor" },
        { type: "fix", release: "patch" },
        { type: "perf", release: "patch" },
        ...["chore", "refactor", "revert"].map(type => ({ type, release: "patch" })),
        // Ignore historical generated version commits.
        { type: "build", scope: "!release", release: "patch" },
        { type: "build", scope: null, release: "patch" },
        { type: "build", scope: "release", release: false },
        ...["ci", "docs", "style", "test"].map(type => ({ type, release: false })),
      ],
    }],
    ["@semantic-release/release-notes-generator", {
      preset: "conventionalcommits",
      parserOpts,
      presetConfig: {
        types: [
          { type: "feat", section: "Features" },
          { type: "fix", section: "Bug Fixes" },
          { type: "perf", section: "Performance Improvements" },
          { type: "chore", section: "Maintenance" },
          { type: "build", scope: "release", hidden: true },
          { type: "build", section: "Build System" },
          { type: "refactor", section: "Code Refactoring" },
          { type: "revert", section: "Reverts" },
          ...["ci", "docs", "style", "test"].map(type => ({ type, hidden: true })),
        ],
      },
    }],
    "./scripts/semantic-release-plugin.mjs",
    ["@semantic-release/github", {
      assets: ["dist/*.tar.gz", "dist/SHA256SUMS", "README.md", "LICENSE"],
      successComment: false,
      failComment: false,
      releasedLabels: false,
    }],
  ],
};
