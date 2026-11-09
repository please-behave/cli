// A semantic-release dry run: no prepare hooks, tags, commits or publication.
import { execFileSync } from "node:child_process";
import { appendFileSync } from "node:fs";
import semanticRelease from "semantic-release";
import config from "../release.config.mjs";

if (!process.env.GITHUB_OUTPUT || process.env.GITHUB_REF !== "refs/heads/main") {
  throw new Error("Release planning is restricted to the main-branch workflow.");
}
const source = execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim();
if (source !== process.env.GITHUB_SHA) {
  throw new Error("Release planning checkout does not match the dispatched source.");
}
const result = await semanticRelease({
  ...config,
  dryRun: true,
  plugins: config.plugins.slice(0, 2),
});
appendFileSync(process.env.GITHUB_OUTPUT,
  `release=${Boolean(result)}\nversion=${result ? result.nextRelease.version : ""}\nsource_sha=${source}\n`);
