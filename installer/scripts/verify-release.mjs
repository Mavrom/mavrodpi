import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { inspectPortableExecutable } from "./pe-utils.mjs";

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
const releasePath = path.resolve(
  scriptDirectory,
  "..",
  "release",
  "MavroDPI-Setup.exe",
);
const packagedPayload = path.resolve(
  scriptDirectory,
  "..",
  "release",
  "win-unpacked",
  "resources",
  "payload",
  "MavroDPI-payload.exe",
);
const packagedManifest = path.resolve(
  scriptDirectory,
  "..",
  "release",
  "win-unpacked",
  "resources",
  "payload",
  "payload-manifest.json",
);
const builtExecutablePath = path.resolve(
  scriptDirectory,
  "..",
  "..",
  "src-tauri",
  "target",
  "release",
  "mavrodpi.exe",
);
const release = await inspectPortableExecutable(releasePath, {
  minBytes: 1024 * 1024,
});
const payload = await inspectPortableExecutable(packagedPayload);
const builtExecutable =
  await inspectPortableExecutable(builtExecutablePath);
const manifest = JSON.parse(await readFile(packagedManifest, "utf8"));

if (
  manifest.schemaVersion !== 1 ||
  manifest.productName !== "MavroDPI" ||
  manifest.version !== "0.3.1" ||
  manifest.architecture !== "x64" ||
  manifest.fileName !== "MavroDPI-payload.exe" ||
  manifest.size !== payload.size ||
  manifest.sha256 !== payload.sha256 ||
  manifest.installedExecutableFileName !== "MavroDPI.exe" ||
  manifest.installedExecutableSize !== builtExecutable.size ||
  manifest.installedExecutableSha256 !== builtExecutable.sha256
) {
  throw new Error(
    "Paketlenmiş payload veya kurulu uygulama kimliği payload-manifest.json ile eşleşmiyor.",
  );
}

console.log(`Release doğrulandı: ${release.path}`);
console.log(`Boyut: ${release.size} bayt`);
console.log(`SHA256: ${release.sha256}`);
console.log(
  `Payload manifesti doğrulandı: ${manifest.version} · ${payload.size} bayt · ${payload.sha256}`,
);
console.log(
  `Kurulu EXE kimliği doğrulandı: ${manifest.installedExecutableFileName} · ${builtExecutable.size} bayt · ${builtExecutable.sha256}`,
);
