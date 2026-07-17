import process from "node:process";
import { pathToFileURL } from "node:url";

import { runLinuxDebPostPublicSmoke } from "./linux-post-public-smoke.mjs";

export { linuxDebPostPublicSmokeInternals } from "./linux-post-public-smoke.mjs";

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  runLinuxDebPostPublicSmoke(process.argv.slice(2));
}
