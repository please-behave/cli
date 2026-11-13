import assert from "node:assert/strict";
import { test } from "node:test";
import { analyzeCommits } from "./release-analyzer.mjs";
import { generateNotes } from "@semantic-release/release-notes-generator";
import config from "../../release.config.mjs";

const logger = { log() {}, error() {} };
const options = config.plugins[0][1];
for (const [message, expected] of [
  ["feat(cli): add a command", "minor"],
  ["fix: correct a verdict", "patch"],
  ["perf: reduce repeated context", "patch"],
  ["feat!: remove a command", "minor"],
  ["refactor(config)!: change configuration", "minor"],
  ["fix: change format\n\nBREAKING CHANGE: old reports need migration", "minor"],
  ["chore(deps): update rust crate toml to v1.2.0", "patch"],
  ["chore(deps)!: drop an older runtime", "minor"],
  ...["build", "chore", "config", "refactor", "revert"].flatMap(type => [
    [`${type}: maintenance`, "patch"],
    [`${type}(cli): maintenance`, "patch"],
  ]),
  ...["ci", "docs", "style", "test"].map(type => [`${type}: maintenance`, null]),
  ...["build", "ci", "config", "docs", "feat", "fix", "perf", "refactor", "revert", "style", "test"].map(type => [
    `${type}!: incompatible change`, "minor",
  ]),
  ["build(release): 1.2.3 [skip ci]", null],
  ["build(release)!: incompatible release tooling", "minor"],
]) {
  test(`release rule: ${message.split("\n")[0]}`, async () => {
    assert.equal(await analyzeCommits(options, {
      cwd: process.cwd(), commits: [{ hash: "abc", message }], logger,
    }), expected);
  });
}
const commit = (message, digit) => ({ hash: digit.repeat(40), message });
const analyze = commits => analyzeCommits(options, { cwd: process.cwd(), commits, logger });
const notesFor = commits => generateNotes(config.plugins[1][1], {
  cwd: process.cwd(), logger,
  options: { repositoryUrl: "https://github.com/please-behave/cli.git" },
  commits,
  lastRelease: { gitTag: "v1.0.0" },
  nextRelease: { gitTag: "v2.0.0", version: "2.0.0" },
});

for (const scope of ["", "(cli)"]) {
  const original = commit("feat(cli): add an experimental command", "a");
  const revert = commit(`revert${scope}: ${original.message}\n\nThis reverts commit ${original.hash}.`, "b");
  test(`revert${scope} of an already released change triggers a patch and appears in notes`, async () => {
    assert.equal(await analyze([revert]), "patch");
    assert.match(await notesFor([revert]), /Reverts/);
  });
  test(`revert${scope} cancels an unreleased original in version calculation and notes`, async () => {
    // Semantic Release supplies Git history newest-first, as required by its revert filter.
    for (const commits of [[revert, original], [commit("docs: explain the change", "d"), revert, original]]) {
      assert.equal(await analyze(commits), null);
      assert.doesNotMatch(await notesFor(commits), /experimental command/);
      assert.equal(await analyze([...commits, commit("fix: preserve another change", "c")]), "patch");
    }
  });
}

test("a release uses the highest version increment across commits", async () => {
  const maintenance = commit("chore(deps): update dependencies", "a");
  const feature = commit("feat(cli): add a command", "b");
  const breaking = commit("docs!: remove supported configuration", "c");
  assert.equal(await analyze([maintenance, feature]), "minor");
  assert.equal(await analyze([maintenance, feature, breaking]), "minor");
});

test("patch maintenance is visible in release notes; internal and cosmetic changes are hidden", async () => {
  const commits = [
    ...["chore", "build", "config", "refactor", "revert"].map((type, index) => commit(`${type}(deps): visible ${type}`, String(index))),
    ...["ci", "docs", "style", "test"].map((type, index) => commit(`${type}: hidden ${type}`, String(index + 5))),
    commit("build(release): 1.2.3 [skip ci]", "g"),
  ];
  const notes = await notesFor(commits);
  for (const type of ["chore", "build", "config", "refactor", "revert"]) assert.match(notes, new RegExp(`visible ${type}`));
  assert.doesNotMatch(notes, /hidden |skip ci/);
});

test("historical version commit with breaking release notes cannot trigger another release", async () => {
  const notes = await notesFor([commit("feat!: change review format", "a")]);
  const template = "build(release): ${nextRelease.version} [skip ci]\n\n${nextRelease.notes}";
  const message = template.replace("${nextRelease.version}", "2.0.0").replace("${nextRelease.notes}", notes);
  assert.equal(await analyze([commit(message, "b")]), null);
});

test("notes preserve features, fixes and breaking changes", async () => {
  const notes = await generateNotes(config.plugins[1][1], {
    cwd: process.cwd(), logger,
    options: { repositoryUrl: "https://github.com/please-behave/cli.git" },
    commits: [{ hash: "a".repeat(40), message: "feat!: change review format" }],
    lastRelease: { gitTag: "v1.0.0" },
    nextRelease: { gitTag: "v2.0.0", version: "2.0.0" },
  });
  assert.match(notes, /BREAKING CHANGES/);
  assert.match(notes, /change review format/);
});
test("release is main-only with private tooling and no npm publication", () => {
  assert.deepEqual(config.branches, ["main"]);
  assert.deepEqual(config.plugins.map(plugin => Array.isArray(plugin) ? plugin[0] : plugin), [
    "./.github/scripts/release-analyzer.mjs", "@semantic-release/release-notes-generator",
    "./.github/scripts/semantic-release-plugin.mjs", "@semantic-release/github",
  ]);
  assert.ok(!config.plugins.some(plugin => JSON.stringify(plugin).includes("@semantic-release/npm")));
  const github = config.plugins.at(-1)[1];
  assert.equal(github.successComment, false);
  assert.equal(github.failComment, false);
});

test("stable graduation is explicit; development policy rejects stable history", async () => {
  const context = { cwd: process.cwd(), commits: [commit("feat!: incompatible change", "a")], logger, lastRelease: { version: "0.1.0" } };
  assert.equal(await analyzeCommits(options, context), "minor");
  assert.equal(await analyzeCommits({ ...options, developmentSeries: false }, context), "major");
  await assert.rejects(analyzeCommits(options, { ...context, lastRelease: { version: "1.0.0" } }), /unexpected stable release tag/);
});
