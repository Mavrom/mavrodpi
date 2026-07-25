import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(scriptDirectory, "..");
const read = (name) => readFile(path.join(root, name), "utf8");

const [
  packageSource,
  mainSource,
  preloadSource,
  htmlSource,
  rendererSource,
  preparePayloadSource,
  verifyReleaseSource,
] =
  await Promise.all([
    read("package.json"),
    read("main.cjs"),
    read("preload.cjs"),
    read("renderer/index.html"),
    read("renderer/app.js"),
    read("scripts/prepare-payload.mjs"),
    read("scripts/verify-release.mjs"),
  ]);
const manifest = JSON.parse(packageSource);

function requireCondition(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}

requireCondition(manifest.version === "0.3.3", "Wrapper sürümü 0.3.3 olmalı.");
requireCondition(
  manifest.build?.artifactName === "MavroDPI-Setup.exe",
  "Release adı MavroDPI-Setup.exe olmalı.",
);
requireCondition(
  manifest.build?.win?.requestedExecutionLevel === "asInvoker",
  "Wrapper başlangıçta yönetici yetkisi istememeli.",
);
requireCondition(
  manifest.build?.portable?.requestExecutionLevel === "user",
  "Portable wrapper kullanıcı yetkisiyle başlamalı.",
);
requireCondition(
  manifest.build?.electronDist === "node_modules/electron/dist",
  "Paketleme doğrulanmış yerel Electron dağıtımını kullanmalı.",
);
requireCondition(
  manifest.scripts?.postinstall === "node node_modules/electron/install.js",
  "npm ci sonrasında yerel Electron dağıtımı hazırlanmalı.",
);
requireCondition(
  JSON.stringify(manifest.build?.extraResources).includes(
    "payload/MavroDPI-payload.exe",
  ),
  "Doğrulanmış NSIS payload pakete eklenmeli.",
);
requireCondition(
  JSON.stringify(manifest.build?.extraResources).includes(
    "payload/payload-manifest.json",
  ),
  "Payload manifesti pakete eklenmeli.",
);

for (const token of [
  "frame: false",
  "transparent: true",
  "contextIsolation: true",
  "sandbox: true",
  "nodeIntegration: false",
  'action: "deny"',
  "will-navigate",
  "setPermissionRequestHandler",
]) {
  requireCondition(mainSource.includes(token), `Güvenlik ayarı eksik: ${token}`);
}
requireCondition(
  !mainSource.includes("nodeIntegration: true"),
  "Node integration açılmamalı.",
);
for (const token of [
  "PAYLOAD_MANIFEST_FILE",
  "manifest?.version !== PRODUCT_VERSION",
  "manifest?.installedExecutableFileName !== INSTALLED_EXECUTABLE",
  "executable.size !== manifest.size",
  "executable.sha256 !== manifest.sha256",
  '"HKLM\\\\SOFTWARE\\\\Microsoft\\\\Windows\\\\CurrentVersion\\\\Uninstall\\\\MavroDPI"',
  "values.displayname.trim().toLowerCase() === PRODUCT_NAME.toLowerCase()",
  "registryApplicationCandidates(entry)",
  "findValidatedInstalledApplication",
  "requiredVersion: PRODUCT_VERSION",
  "detectExistingMavroInstall",
  'const installMode = existingRecord ? "repair" : "install"',
  'installMode === "repair"',
  "executable.size !== expectedExecutableSize",
  "executable.sha256 !== expectedExecutableSha256",
  "candidateInsideProgramFiles(executable.path, roots)",
  "MAVRODPI_UPDATE_MODE",
  "installerArguments += '/UPDATE'",
]) {
  requireCondition(
    mainSource.includes(token),
    `Runtime bütünlük doğrulaması eksik: ${token}`,
  );
}
for (const token of [
  'nsis_tauri_utils::KillProcess "${MAINBINARYNAME}.exe"',
  "SetOverwrite on",
]) {
  requireCondition(
    (await read("../src-tauri/nsis/hooks.nsi")).includes(token),
    `Onarım sırasında dosya değişimi güvence altına alınmalı: ${token}`,
  );
}
for (const token of [
  "schemaVersion: 1",
  'productName: "MavroDPI"',
  "version: VERSION",
  'architecture: "x64"',
  "size: copiedInfo.size",
  "sha256: copiedInfo.sha256",
  'installedExecutableFileName: "MavroDPI.exe"',
  "installedExecutableSize: installedExecutable.size",
  "installedExecutableSha256: installedExecutable.sha256",
]) {
  requireCondition(
    preparePayloadSource.includes(token),
    `Payload manifest üretimi eksik: ${token}`,
  );
}
for (const token of [
  'manifest.installedExecutableFileName !== "MavroDPI.exe"',
  "manifest.installedExecutableSize !== builtExecutable.size",
  "manifest.installedExecutableSha256 !== builtExecutable.sha256",
]) {
  requireCondition(
    verifyReleaseSource.includes(token),
    `Release içindeki kurulu EXE kimliği doğrulaması eksik: ${token}`,
  );
}
requireCondition(
  preloadSource.includes("contextBridge.exposeInMainWorld"),
  "Preload yalnız dar bir contextBridge API'si sunmalı.",
);

for (const directive of [
  "default-src 'self'",
  "script-src 'self'",
  "style-src 'self'",
  "connect-src 'none'",
  "object-src 'none'",
  "frame-src 'none'",
  "base-uri 'none'",
]) {
  requireCondition(htmlSource.includes(directive), `CSP eksik: ${directive}`);
}
requireCondition(
  !/<script(?![^>]*\bsrc=)[^>]*>/i.test(htmlSource),
  "Inline script kullanılamaz.",
);
requireCondition(
  !/<style(?:\s|>)/i.test(htmlSource),
  "Inline stil kullanılamaz.",
);
requireCondition(
  !/https?:\/\//i.test(htmlSource + rendererSource),
  "Renderer harici ağ kaynağı içeremez.",
);
requireCondition(
  !/\bimzalı\b/i.test(htmlSource + rendererSource),
  "Kod imzası bulunmadığı için arayüz imzalı kurulum iddiası taşıyamaz.",
);

console.log("Statik güvenlik ve paket yapılandırması doğrulandı.");
