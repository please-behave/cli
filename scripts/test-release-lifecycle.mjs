// Exercise semantic release planning and publication offline.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { chmodSync, copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { createRequire } from "node:module";
import { Writable } from "node:stream";
import semanticRelease from "semantic-release";
import config from "../release.config.mjs";
import { verifyRelease } from "./semantic-release-plugin.mjs";

const require = createRequire(import.meta.url);
const source = resolve(import.meta.dirname, "..");
assert.throws(() => verifyRelease({}, {
  env: { EXPECTED_RELEASE_VERSION: "0.1.1" }, nextRelease: { version: "0.2.0" },
}), /version changed/);

async function testLifecycle() {
  const temporary = mkdtempSync(join(tmpdir(), "please-release-test-"));
  const cwd = join(temporary, "repository");
  const remote = join(temporary, "remote.git");
  mkdirSync(cwd);
  const env = { ...process.env, EXPECTED_RELEASE_VERSION: "0.1.0" };
  for (const key of Object.keys(env)) {
    if (key.startsWith("GITHUB_") || ["CI", "GH_TOKEN", "GITLAB_CI"].includes(key)) delete env[key];
  }
  const git = (...args) => execFileSync("git", args, { cwd, env, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();
  const quiet = new Writable({ write(_chunk, _encoding, done) { done(); } });
  try {
    git("init", "--bare", "--initial-branch=main", remote);
    git("init", "--initial-branch=main");
    git("config", "user.name", "Release Fixture");
    git("config", "user.email", "fixture@example.invalid");
    git("config", "commit.gpgsign", "false");
    git("config", "tag.gpgsign", "false");
    git("remote", "add", "origin", remote);
    for (const file of ["README.md", "LICENSE", "Cargo.toml", "Cargo.lock"]) copyFileSync(join(source, file), join(cwd, file));
    mkdirSync(join(cwd, "scripts"));
    for (const file of ["verify-release.py", "check-release-artifacts.py", "set-release-version.py"]) copyFileSync(join(source, "scripts", file), join(cwd, "scripts", file));
    writeFileSync(join(cwd, ".gitignore"), "dist/\n");
    git("add", "."); git("commit", "-m", "feat: initialize fixture"); git("push", "--set-upstream", "origin", "main");
    env.GITHUB_SHA = git("rev-parse", "HEAD");
    function archives(version) {
      env.EXPECTED_RELEASE_VERSION = version;
      rmSync(join(cwd, "dist"), { recursive: true, force: true });
      execFileSync("python3", ["-c", `
import hashlib, io, json, os, pathlib, tarfile
version=os.environ['EXPECTED_RELEASE_VERSION']
dist=pathlib.Path('dist'); (dist/'metadata').mkdir(parents=True)
targets=['aarch64-apple-darwin','x86_64-apple-darwin','x86_64-unknown-linux-gnu','aarch64-unknown-linux-gnu']
for target in targets:
    name=f'please-v{version}-{target}'
    binary=f'fixture executable for {target}'.encode()
    archive=dist/f'{name}.tar.gz'
    with tarfile.open(archive,'w:gz') as output:
        for file,data in [('please',binary),('README.md',pathlib.Path('README.md').read_bytes()),('LICENSE',pathlib.Path('LICENSE').read_bytes())]:
            member=tarfile.TarInfo(f'{name}/{file}');member.size=len(data);member.mode=0o755 if file=='please' else 0o644
            output.addfile(member,io.BytesIO(data))
    (dist/'metadata'/f'{target}.json').write_text(json.dumps(dict(version=version,source_sha=os.environ['GITHUB_SHA'],target=target,binary_version=f'please {version}',archive_sha256=hashlib.sha256(archive.read_bytes()).hexdigest(),binary_sha256=hashlib.sha256(binary).hexdigest())))
(dist/'SHA256SUMS').write_text(''.join(f'{hashlib.sha256(p.read_bytes()).hexdigest()}  {p.name}\\n' for p in sorted(dist.glob('*.tar.gz'))))
`], { cwd, env });
    }
    git("tag", "v0.1.0");
    git("push", "origin", "v0.1.0");
    const verifyArtifacts = () => execFileSync("python3", ["scripts/check-release-artifacts.py"], { cwd, env });
    git("commit", "--allow-empty", "-m", "fix: improve fixture"); git("push", "origin", "main");
    env.GITHUB_SHA = git("rev-parse", "HEAD"); archives("0.1.1");
    const notesPlugin = join(temporary, "notes.mjs");
    writeFileSync(notesPlugin, 'export function generateNotes() { return "Fixture release notes"; }\n');
    const plugins = config.plugins.slice(0, -1).map(plugin => {
      const [name, options] = Array.isArray(plugin) ? plugin : [plugin, {}];
      const path = name === "@semantic-release/release-notes-generator" ? notesPlugin
        : name.startsWith(".") ? resolve(source, name) : require.resolve(name);
      return [path, options];
    });
    const options = { ...config, repositoryUrl: remote, ci: false, plugins };
    const context = { cwd, env, stdout: quiet, stderr: quiet };
    // Fail if publication tries to create a commit.
    for (const event of ["pre-commit", "commit-msg"]) {
      const path = join(cwd, ".git", "hooks", event);
      writeFileSync(path, '#!/bin/sh\necho "Publication must not create commits" >&2\nexit 1\n');
      chmodSync(path, 0o755);
    }
    const original = readFileSync(join(cwd, "Cargo.toml"), "utf8");
    const originalLock = readFileSync(join(cwd, "Cargo.lock"), "utf8");
    const preview = await semanticRelease({ ...options, dryRun: true }, context);
    assert.equal(preview.nextRelease.version, "0.1.1"); assert.equal(git("tag"), "v0.1.0");
    assert.equal(readFileSync(join(cwd, "Cargo.toml"), "utf8"), original);
    await assert.rejects(semanticRelease(options, { ...context, env: { ...env, EXPECTED_RELEASE_VERSION: "9.9.9" } }));
    assert.equal(git("tag"), "v0.1.0"); assert.equal(git("rev-parse", "HEAD"), env.GITHUB_SHA);
    const checksums = readFileSync(join(cwd, "dist", "SHA256SUMS"), "utf8");
    writeFileSync(join(cwd, "dist", "SHA256SUMS"), "corrupt checksums\n");
    assert.throws(verifyArtifacts);
    assert.equal(git("tag"), "v0.1.0");
    assert.equal(git("rev-parse", "HEAD"), env.GITHUB_SHA);
    writeFileSync(join(cwd, "dist", "SHA256SUMS"), checksums);
    await assert.rejects(semanticRelease(options, { ...context, env: { ...env, GITHUB_SHA: "0".repeat(40) } }));
    assert.equal(git("tag"), "v0.1.0");
    verifyArtifacts();
    const result = await semanticRelease(options, context);
    assert.equal(result.nextRelease.version, "0.1.1");
    assert.equal(readFileSync(join(cwd, "Cargo.toml"), "utf8"), original);
    assert.equal(readFileSync(join(cwd, "Cargo.lock"), "utf8"), originalLock);
    assert.equal(git("status", "--porcelain"), "");
    assert.equal(git("rev-parse", "HEAD"), env.GITHUB_SHA);
    assert.equal(git("rev-parse", "v0.1.1"), env.GITHUB_SHA);
    assert.equal(git("ls-remote", "origin", "refs/heads/main").split(/\s+/)[0], env.GITHUB_SHA);
    assert.match(git("ls-remote", "origin", "refs/tags/v0.1.1"), /refs\/tags\/v0\.1\.1/);
    assert.match(git("ls-remote", "origin", "refs/notes/semantic-release-v0.1.1"), /refs\/notes\/semantic-release-v0\.1\.1/);
    assert.equal(await semanticRelease(options, context), false);
    for (const event of ["pre-commit", "commit-msg"]) rmSync(join(cwd, ".git", "hooks", event));
    git("commit", "--allow-empty", "-m", "feat!: change fixture contract"); git("push", "origin", "main");
    assert.equal((await semanticRelease({ ...options, plugins: plugins.slice(0, 2), dryRun: true }, context)).nextRelease.version, "0.2.0");
  } finally { rmSync(temporary, { recursive: true, force: true }); }
}
await testLifecycle();
console.log("Release lifecycle passed: inert preview, patch 0.1.1 and breaking 0.2.0.");
