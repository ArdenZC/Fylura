import { mkdir, readFile, stat, writeFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { unzipSync } from "fflate";

const ZIP_RELATIVE = "docs/design/prototypes/Fylura-proto-v3.40.zip";
const ROOT_NAME = "fylura-proto-v3.40";

/**
 * Extract the committed v3.40 prototype next to the OS temp directory.
 * Every consumer reads the ZIP from the repository; no machine-specific path.
 */
export async function ensureProto340(repoRoot = process.cwd()) {
  const zipPath = path.join(repoRoot, ZIP_RELATIVE);
  const targetRoot = path.join(os.tmpdir(), "fylura-proto-v3.40");
  const entry = path.join(targetRoot, "redesign-v2.html");
  if (existsSync(entry)) return { root: targetRoot, html: entry };

  const zip = unzipSync(new Uint8Array(await readFile(zipPath)));
  await mkdir(targetRoot, { recursive: true });
  for (const [name, bytes] of Object.entries(zip)) {
    const relative = name.startsWith(`${ROOT_NAME}/`) ? name.slice(ROOT_NAME.length + 1) : name;
    if (!relative || relative.endsWith("/")) continue;
    const target = path.join(targetRoot, relative);
    await mkdir(path.dirname(target), { recursive: true });
    await writeFile(target, bytes);
  }
  const info = await stat(entry).catch(() => null);
  if (!info) throw new Error(`v3.40 prototype entry missing: ${entry}`);
  return { root: targetRoot, html: entry };
}
