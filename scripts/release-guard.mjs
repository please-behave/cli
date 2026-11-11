import { execFileSync } from "node:child_process";
import { pathToFileURL } from "node:url";

export function verifySource(env, cwd, git = execFileSync) {
  if (env.GITHUB_REF !== "refs/heads/main" || !env.GITHUB_REPOSITORY
      || env.GITHUB_REPOSITORY !== "please-behave/cli"
      || !/^[0-9a-f]{40}$/.test(env.GITHUB_SHA ?? "")) {
    throw new Error("Invalid publication context");
  }
  const execution = { cwd, env, encoding: "utf8" };
  const head = git("git", ["rev-parse", "HEAD"], execution).trim();
  const remote = git("git", ["ls-remote", "origin", "refs/heads/main"], execution).trim().split(/\s+/)[0];
  if (head !== env.GITHUB_SHA || remote !== head) {
    throw new Error("main changed during the release; rerun from its latest commit.");
  }
}

export function verifyRelease(_config, context) {
  const env = context.env ?? process.env;
  if (context.nextRelease.version !== env.EXPECTED_RELEASE_VERSION) {
    throw new Error("Release version changed after planning; refuse publication.");
  }
  verifySource(env, context.cwd);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  if (process.argv.slice(2).join(" ") !== "--source") throw new Error("Usage: release-guard.mjs --source");
  verifySource(process.env, process.cwd());
}
