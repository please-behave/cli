import { analyzeCommits as conventionalAnalysis } from "@semantic-release/commit-analyzer";

export async function analyzeCommits({ developmentSeries = true, ...options }, context) {
  if (developmentSeries && context.lastRelease?.version && !context.lastRelease.version.startsWith("0.")) {
    throw new Error("Please Behave is in the 0.x development series; resolve the unexpected stable release tag before publishing.");
  }
  const type = await conventionalAnalysis(options, context);
  return developmentSeries && type === "major" ? "minor" : type;
}
