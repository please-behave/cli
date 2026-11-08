// A semantic-release dry run: no prepare hooks, tags, commits or publication.
import { appendFileSync } from "node:fs";
import semanticRelease from "semantic-release";
import config from "../release.config.mjs";
import { INITIAL_VERSION, isInitialRelease, verifyInitialSource } from "./initial-release.mjs";

if (!process.env.GITHUB_OUTPUT || process.env.GITHUB_REF !== "refs/heads/main") {
  throw new Error("Release planning is restricted to the main-branch workflow.");
}
const context = { cwd: process.cwd(), env: process.env };
const initial = isInitialRelease(context);
if (initial) verifyInitialSource(context);
const result = initial ? { nextRelease: { version: INITIAL_VERSION } } : await semanticRelease({
  ...config,
  dryRun: true,
  plugins: config.plugins.slice(0, 2),
});
appendFileSync(process.env.GITHUB_OUTPUT,
  `release=${Boolean(result)}\ninitial=${initial}\nversion=${result ? result.nextRelease.version : ""}\n`);
