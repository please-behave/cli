import semanticRelease from "semantic-release";
import config from "../release.config.mjs";
import { publishInitialRelease } from "./initial-release.mjs";

if (process.env.GITHUB_REF !== "refs/heads/main" || !process.env.EXPECTED_RELEASE_VERSION) {
  throw new Error("Publication is restricted to the main-branch release workflow.");
}
if (process.env.INITIAL_RELEASE === "true") {
  await publishInitialRelease({ cwd: process.cwd(), env: process.env });
} else {
  await semanticRelease(config);
}
