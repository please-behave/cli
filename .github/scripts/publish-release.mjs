import semanticRelease from "semantic-release";
import config from "../../release.config.mjs";

if (process.env.GITHUB_REF !== "refs/heads/main" || !process.env.EXPECTED_RELEASE_VERSION) {
  throw new Error("Publication is restricted to the main-branch release workflow.");
}
await semanticRelease(config);
