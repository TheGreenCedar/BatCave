import process from "node:process";
import { pathToFileURL } from "node:url";

import { runLinuxAppImagePostPublicSmoke } from "./linux-post-public-smoke.mjs";

export { linuxAppImagePostPublicSmokeInternals } from "./linux-post-public-smoke.mjs";

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  runLinuxAppImagePostPublicSmoke(process.argv.slice(2));
}
