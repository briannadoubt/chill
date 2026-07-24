import { execFileSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const temp = mkdtempSync(join(tmpdir(), "chill-runtime-pack-"));
let packageFile;
try {
  const tarball = execFileSync("npm", ["pack", "--json"], { encoding: "utf8" });
  packageFile = JSON.parse(tarball).at(0).filename;
  execFileSync("tar", ["-xzf", packageFile, "-C", temp]);
  const root = join(temp, "package", "dist");
  for (const [host, command, args] of [["node", "node", []], ["deno", "deno", ["run", "--allow-read"]], ["bun", "bun", []]]) {
    const entry = pathToFileURL(join(root, `${host}.js`)).href;
    execFileSync(command, [...args, "scripts/smoke.mjs", host, entry], { stdio: "inherit" });
  }
} finally { rmSync(temp, { recursive: true, force: true }); if (packageFile) rmSync(packageFile, { force: true }); }
