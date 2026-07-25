import {
  copyFile,
  mkdir,
  readdir,
  realpath,
  rename,
  rm,
  writeFile,
} from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { inspectPortableExecutable } from "./pe-utils.mjs";

const VERSION = "0.3.1";
const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
const installerRoot = path.resolve(scriptDirectory, "..");
const nsisDirectory = path.resolve(
  installerRoot,
  "..",
  "src-tauri",
  "target",
  "release",
  "bundle",
  "nsis",
);
const payloadDirectory = path.join(installerRoot, "payload");
const payloadPath = path.join(payloadDirectory, "MavroDPI-payload.exe");
const temporaryPayload = `${payloadPath}.tmp`;
const manifestPath = path.join(payloadDirectory, "payload-manifest.json");
const temporaryManifest = `${manifestPath}.tmp`;
const installedExecutablePath = path.resolve(
  installerRoot,
  "..",
  "src-tauri",
  "target",
  "release",
  "mavrodpi.exe",
);
const expectedName = new RegExp(
  `^MavroDPI_${VERSION.replaceAll(".", "\\.")}_x64-setup\\.exe$`,
  "i",
);

async function findPayload() {
  let entries;
  try {
    entries = await readdir(nsisDirectory, { withFileTypes: true });
  } catch {
    throw new Error(
      `Tauri NSIS çıktısı bulunamadı. Önce ana projede "npm run tauri build" çalıştırılmalı: ${nsisDirectory}`,
    );
  }

  const matches = entries.filter(
    (entry) => entry.isFile() && expectedName.test(entry.name),
  );
  if (matches.length !== 1) {
    throw new Error(
      `MavroDPI ${VERSION} x64 NSIS paketi tekil olarak bulunamadı (${matches.length} eşleşme): ${nsisDirectory}`,
    );
  }

  const directoryRealPath = await realpath(nsisDirectory);
  const candidate = path.join(nsisDirectory, matches[0].name);
  const candidateRealPath = await realpath(candidate);
  if (path.dirname(candidateRealPath) !== directoryRealPath) {
    throw new Error("NSIS payload yolu beklenen çıktı klasörünün dışına çıkıyor.");
  }

  return candidateRealPath;
}

const sourcePayload = await findPayload();
const sourceInfo = await inspectPortableExecutable(sourcePayload);
const installedExecutable = await inspectPortableExecutable(
  installedExecutablePath,
);
const releaseDirectoryRealPath = await realpath(
  path.dirname(installedExecutablePath),
);
if (
  path.dirname(installedExecutable.path) !== releaseDirectoryRealPath ||
  path.basename(installedExecutable.path).toLowerCase() !== "mavrodpi.exe"
) {
  throw new Error(
    "Kurulu uygulama kimliği için kullanılan mavrodpi.exe beklenen release klasörünün dışında.",
  );
}

await mkdir(payloadDirectory, { recursive: true });
await rm(temporaryPayload, { force: true });
await copyFile(sourceInfo.path, temporaryPayload);
await inspectPortableExecutable(temporaryPayload);
await rm(payloadPath, { force: true });
await rename(temporaryPayload, payloadPath);

const copiedInfo = await inspectPortableExecutable(payloadPath);
if (copiedInfo.sha256 !== sourceInfo.sha256 || copiedInfo.size !== sourceInfo.size) {
  throw new Error("Kopyalanan payload kaynak NSIS paketiyle eşleşmiyor.");
}

const payloadManifest = {
  schemaVersion: 1,
  productName: "MavroDPI",
  version: VERSION,
  architecture: "x64",
  fileName: path.basename(payloadPath),
  size: copiedInfo.size,
  sha256: copiedInfo.sha256,
  installedExecutableFileName: "MavroDPI.exe",
  installedExecutableSize: installedExecutable.size,
  installedExecutableSha256: installedExecutable.sha256,
};
await rm(temporaryManifest, { force: true });
await writeFile(
  temporaryManifest,
  `${JSON.stringify(payloadManifest, null, 2)}\n`,
  "utf8",
);
await rm(manifestPath, { force: true });
await rename(temporaryManifest, manifestPath);

console.log(`Payload hazır: ${path.basename(sourcePayload)}`);
console.log(`Boyut: ${sourceInfo.size} bayt`);
console.log(`SHA256: ${sourceInfo.sha256}`);
console.log(`Manifest: ${manifestPath}`);
