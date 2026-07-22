import { copyFile, mkdir, readdir, rm } from "node:fs/promises";

const dist = new URL("../dist/", import.meta.url);
for (const entry of await readdir(dist, { withFileTypes: true })) {
  if (entry.name === "client") continue;
  await rm(new URL(entry.name, dist), { force: true, recursive: entry.isDirectory() });
}

for (const asset of ["favicon.svg", "chill-observability-og.png"]) {
  await copyFile(
    new URL(`../public/${asset}`, import.meta.url),
    new URL(`../dist/client/${asset}`, import.meta.url),
  );
}

await mkdir(new URL("../dist/server/", import.meta.url), { recursive: true });
await copyFile(
  new URL("../sites-worker.js", import.meta.url),
  new URL("../dist/server/index.js", import.meta.url),
);
await mkdir(new URL("../dist/.openai/", import.meta.url), { recursive: true });
await copyFile(
  new URL("../.openai/hosting.json", import.meta.url),
  new URL("../dist/.openai/hosting.json", import.meta.url),
);
