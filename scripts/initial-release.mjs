// semantic-release starts at 1.0.0. Bootstrap the initial 0.1.0 through the same
// publication guard, after workflow artifact verification. Subsequent versions
// are calculated by semantic-release.
import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { generateNotes } from "@semantic-release/release-notes-generator";
import config from "../release.config.mjs";
import { verifyRelease } from "./semantic-release-plugin.mjs";

export const INITIAL_VERSION = "0.1.0";
const git = (context, ...args) => execFileSync("git", args, {
  cwd: context.cwd, env: context.env ?? process.env, encoding: "utf8",
}).trim();

export function isInitialRelease(context) {
  return !git(context, "tag", "--merged", "HEAD", "--list", "v*").split("\n")
    .some(tag => /^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(tag));
}
export function verifyInitialSource(context) {
  if (!isInitialRelease(context)) throw new Error("Initial release requires no existing semantic release tag.");
  const manifest = readFileSync(join(context.cwd, "Cargo.toml"), "utf8");
  const version = /^version\s*=\s*"([^"]+)"/m.exec(manifest.split("[package]")[1]?.split(/\n\[/)[0] ?? "")?.[1];
  if (version !== INITIAL_VERSION) throw new Error(`Initial source version must be ${INITIAL_VERSION}.`);
}
export async function publishInitialRelease(context, publish = execFileSync) {
  const env = context.env ?? process.env;
  if (env.EXPECTED_RELEASE_VERSION !== INITIAL_VERSION || !env.GITHUB_SHA) {
    throw new Error("Initial publication requires the planned 0.1.0 version and source SHA.");
  }
  verifyInitialSource(context);
  verifyRelease({}, { ...context, nextRelease: { version: INITIAL_VERSION } });
  const hashes = git(context, "rev-list", "HEAD").split("\n");
  const commits = hashes.map(hash => ({ hash, message: git(context, "show", "-s", "--format=%B", hash) }));
  const gitTag = `v${INITIAL_VERSION}`;
  const notes = await generateNotes(config.plugins[1][1], {
    ...context, commits, lastRelease: {}, nextRelease: { version: INITIAL_VERSION, gitTag },
    options: { repositoryUrl: git(context, "remote", "get-url", "origin"), ...context.options },
    logger: context.logger ?? { log() {}, error() {} },
  });
  const temporary = mkdtempSync(join(tmpdir(), "please-release-notes-"));
  try {
    const notesFile = join(temporary, "notes.md");
    writeFileSync(notesFile, notes);
    const archives = readdirSync(join(context.cwd, "dist")).filter(name => name.endsWith(".tar.gz")).sort();
    publish("gh", ["release", "create", gitTag, "--target", env.GITHUB_SHA,
      "--title", gitTag, "--notes-file", notesFile, "--latest",
      ...archives.map(name => `dist/${name}`), "dist/SHA256SUMS", "README.md", "LICENSE"],
    { cwd: context.cwd, env, stdio: "inherit" });
  } finally { rmSync(temporary, { recursive: true, force: true }); }
  return { version: INITIAL_VERSION, gitTag, notes };
}
