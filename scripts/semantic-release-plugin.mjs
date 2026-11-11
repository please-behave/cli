import { createRequire } from "node:module";

const require = createRequire(import.meta.url);

export function verifyRelease(config, context) {
  const env = context.env ?? process.env;
  if (!env.PLEASE_RELEASE_GUARD) throw new Error("Run release-verify before publication");
  require(env.PLEASE_RELEASE_GUARD).verifyRelease(config, context);
}
