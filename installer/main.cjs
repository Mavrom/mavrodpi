"use strict";

const { app, BrowserWindow, ipcMain, session } = require("electron");
const { createHash } = require("node:crypto");
const { execFile, spawn } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { promisify } = require("node:util");

const execFileAsync = promisify(execFile);
const PRODUCT_NAME = "MavroDPI";
const PRODUCT_VERSION = "0.3.3";
const INSTALLED_EXECUTABLE = "MavroDPI.exe";
const PAYLOAD_FILE = "MavroDPI-payload.exe";
const PAYLOAD_MANIFEST_FILE = "payload-manifest.json";
const MIN_PAYLOAD_BYTES = 128 * 1024;
const MIN_INSTALLED_BYTES = 128 * 1024;

const POWERSHELL_SOURCE = String.raw`
$ErrorActionPreference = 'Stop'
$payloadPath = [Environment]::GetEnvironmentVariable('MAVRODPI_PAYLOAD_PATH', 'Process')
$statusPath = [Environment]::GetEnvironmentVariable('MAVRODPI_STATUS_PATH', 'Process')
$resultPath = [Environment]::GetEnvironmentVariable('MAVRODPI_RESULT_PATH', 'Process')
$updateMode = [Environment]::GetEnvironmentVariable('MAVRODPI_UPDATE_MODE', 'Process')
$utf8 = New-Object System.Text.UTF8Encoding($false)

try {
  if ([string]::IsNullOrWhiteSpace($payloadPath) -or
      [string]::IsNullOrWhiteSpace($statusPath) -or
      [string]::IsNullOrWhiteSpace($resultPath)) {
    throw 'Kurulum süreci için gerekli sabit yollar sağlanmadı.'
  }

  $installerArguments = @('/S')
  if ($updateMode -eq '1') {
    $installerArguments += '/UPDATE'
  }
  $installer = Start-Process -FilePath $payloadPath -ArgumentList $installerArguments -Verb RunAs -PassThru

  [System.IO.File]::WriteAllText($statusPath, 'started', $utf8)
  $installer.WaitForExit()
  [System.IO.File]::WriteAllText(
    $resultPath,
    ('EXIT:' + [string]$installer.ExitCode),
    $utf8
  )
  exit $installer.ExitCode
}
catch {
  $message = [Convert]::ToBase64String(
    [System.Text.Encoding]::UTF8.GetBytes($_.Exception.Message)
  )
  [System.IO.File]::WriteAllText($resultPath, ('ERROR:' + $message), $utf8)
  exit 1
}
`;
const POWERSHELL_COMMAND = Buffer.from(
  POWERSHELL_SOURCE,
  "utf16le",
).toString("base64");

let mainWindow = null;
let installationBusy = false;
let allowWindowClose = false;
let lastVerifiedInstall = null;
let currentStatus = {
  phase: "ready",
  message: "Kurulum başlatılmaya hazır.",
  exitCode: null,
  installedPath: null,
  registryKey: null,
  installedVersion: null,
};

function safeErrorMessage(error, fallback = "Beklenmeyen bir hata oluştu.") {
  return error && typeof error.message === "string" && error.message.trim()
    ? error.message.trim()
    : fallback;
}

function emitStatus(nextStatus) {
  currentStatus = {
    ...currentStatus,
    ...nextStatus,
    at: new Date().toISOString(),
  };

  if (mainWindow && !mainWindow.isDestroyed()) {
    mainWindow.webContents.send("installer:status", currentStatus);
  }
}

function isPathInside(parentPath, candidatePath) {
  const relative = path.relative(parentPath, candidatePath);
  return (
    relative !== "" &&
    !relative.startsWith(`..${path.sep}`) &&
    relative !== ".." &&
    !path.isAbsolute(relative)
  );
}

async function inspectPeFile(
  filePath,
  { minBytes, allowedRoot = null } = {},
) {
  const resolvedPath = await fs.promises.realpath(filePath);
  if (allowedRoot) {
    const resolvedRoot = await fs.promises.realpath(allowedRoot);
    if (!isPathInside(resolvedRoot, resolvedPath)) {
      throw new Error("Doğrulanan dosya izin verilen paket yolunun dışında.");
    }
  }

  const fileStats = await fs.promises.stat(resolvedPath);
  if (!fileStats.isFile() || fileStats.size < minBytes) {
    throw new Error(
      `Dosya boyutu güvenlik eşiğini karşılamıyor (${fileStats.size} bayt).`,
    );
  }

  const handle = await fs.promises.open(resolvedPath, "r");
  try {
    const dosHeader = Buffer.alloc(64);
    const dosRead = await handle.read(dosHeader, 0, dosHeader.length, 0);
    if (
      dosRead.bytesRead !== dosHeader.length ||
      dosHeader[0] !== 0x4d ||
      dosHeader[1] !== 0x5a
    ) {
      throw new Error("Payload geçerli bir MZ yürütülebilir dosyası değil.");
    }

    const peOffset = dosHeader.readUInt32LE(0x3c);
    if (peOffset < 0x40 || peOffset > fileStats.size - 4) {
      throw new Error("Payload PE başlık konumu geçersiz.");
    }

    const peSignature = Buffer.alloc(4);
    const peRead = await handle.read(
      peSignature,
      0,
      peSignature.length,
      peOffset,
    );
    if (
      peRead.bytesRead !== peSignature.length ||
      !peSignature.equals(Buffer.from([0x50, 0x45, 0x00, 0x00]))
    ) {
      throw new Error("Payload geçerli bir PE imzası taşımıyor.");
    }
  } finally {
    await handle.close();
  }

  const digest = createHash("sha256");
  const stream = fs.createReadStream(resolvedPath);
  for await (const chunk of stream) {
    digest.update(chunk);
  }

  return {
    path: resolvedPath,
    size: fileStats.size,
    sha256: digest.digest("hex"),
  };
}

function packagedPayloadPath() {
  const root = app.isPackaged ? process.resourcesPath : __dirname;
  return {
    root,
    path: path.join(root, "payload", PAYLOAD_FILE),
    manifestPath: path.join(root, "payload", PAYLOAD_MANIFEST_FILE),
  };
}

async function readPayloadManifest(payload) {
  const resolvedRoot = await fs.promises.realpath(payload.root);
  const manifestPath = await fs.promises.realpath(payload.manifestPath);
  if (!isPathInside(resolvedRoot, manifestPath)) {
    throw new Error("Payload manifesti izin verilen paket yolunun dışında.");
  }

  const manifestStats = await fs.promises.stat(manifestPath);
  if (
    !manifestStats.isFile() ||
    manifestStats.size < 32 ||
    manifestStats.size > 16 * 1024
  ) {
    throw new Error("Payload manifestinin boyutu geçersiz.");
  }

  let manifest;
  try {
    manifest = JSON.parse(await fs.promises.readFile(manifestPath, "utf8"));
  } catch {
    throw new Error("Payload manifesti geçerli JSON değil.");
  }

  if (
    manifest?.schemaVersion !== 1 ||
    manifest?.productName !== PRODUCT_NAME ||
    manifest?.version !== PRODUCT_VERSION ||
    manifest?.architecture !== "x64" ||
    manifest?.fileName !== PAYLOAD_FILE ||
    !Number.isSafeInteger(manifest?.size) ||
    manifest.size < MIN_PAYLOAD_BYTES ||
    typeof manifest?.sha256 !== "string" ||
    !/^[a-f0-9]{64}$/.test(manifest.sha256) ||
    !Number.isSafeInteger(manifest?.installedExecutableSize) ||
    manifest.installedExecutableSize < MIN_INSTALLED_BYTES ||
    typeof manifest?.installedExecutableSha256 !== "string" ||
    !/^[a-f0-9]{64}$/.test(manifest.installedExecutableSha256) ||
    manifest?.installedExecutableFileName !== INSTALLED_EXECUTABLE
  ) {
    throw new Error("Payload manifesti beklenen ürün ve sürümle eşleşmiyor.");
  }

  return manifest;
}

async function inspectPackagedPayload() {
  const payload = packagedPayloadPath();
  const manifest = await readPayloadManifest(payload);
  const executable = await inspectPeFile(payload.path, {
    minBytes: MIN_PAYLOAD_BYTES,
    allowedRoot: payload.root,
  });
  if (
    executable.size !== manifest.size ||
    executable.sha256 !== manifest.sha256
  ) {
    throw new Error(
      "Paket içindeki NSIS dosyası payload manifestiyle eşleşmiyor.",
    );
  }

  return {
    ...executable,
    version: manifest.version,
    architecture: manifest.architecture,
    installedExecutableFileName: manifest.installedExecutableFileName,
    installedExecutableSize: manifest.installedExecutableSize,
    installedExecutableSha256: manifest.installedExecutableSha256,
  };
}

function systemExecutable(name) {
  const windowsRoot = process.env.SystemRoot || process.env.WINDIR || "C:\\Windows";
  return path.join(windowsRoot, "System32", name);
}

async function runRegistryQuery(args) {
  const result = await execFileAsync(systemExecutable("reg.exe"), args, {
    windowsHide: true,
    encoding: "utf8",
    maxBuffer: 2 * 1024 * 1024,
  });
  return result.stdout;
}

function parseRegistryValues(source) {
  const values = {};
  for (const line of source.split(/\r?\n/)) {
    const match = line.match(/^\s+([^\s]+)\s+REG_[A-Z0-9_]+\s+(.*)$/i);
    if (match) {
      values[match[1].toLowerCase()] = match[2].trim();
    }
  }
  return values;
}

async function findMavroRegistryEntries() {
  const roots = [
    {
      query:
        "HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\MavroDPI",
      canonical:
        "HKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\MavroDPI",
    },
    {
      query:
        "HKLM\\SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\MavroDPI",
      canonical:
        "HKEY_LOCAL_MACHINE\\SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\MavroDPI",
    },
    {
      query:
        "HKCU\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\MavroDPI",
      canonical:
        "HKEY_CURRENT_USER\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\MavroDPI",
    },
  ];
  const entries = [];

  for (const root of roots) {
    try {
      const values = parseRegistryValues(
        await runRegistryQuery(["query", root.query]),
      );
      if (
        typeof values.displayname === "string" &&
        values.displayname.trim().toLowerCase() === PRODUCT_NAME.toLowerCase()
      ) {
        entries.push({ key: root.canonical, values });
      }
    } catch {
      continue;
    }
  }

  return entries;
}

async function detectExistingMavroInstall() {
  // Only a real PE under Program Files that is referenced by the exact MavroDPI
  // uninstall entry can enable repair/update mode.
  return findValidatedInstalledApplication();
}

function registryExecutable(value) {
  if (typeof value !== "string") {
    return null;
  }

  const trimmed = value.trim();
  const quoted = trimmed.match(/^"([^"]+\.exe)"/i);
  if (quoted) {
    return quoted[1];
  }

  return trimmed.replace(/,\d+\s*$/, "").trim();
}

function registryDirectory(value) {
  if (typeof value !== "string") {
    return null;
  }

  const trimmed = value.trim();
  const quoted = trimmed.match(/^"([^"]+)"$/);
  const directory = (quoted ? quoted[1] : trimmed).trim();
  return directory && !directory.includes("\0") ? directory : null;
}

function programFilesRoots() {
  return [
    process.env.ProgramFiles,
    process.env["ProgramFiles(x86)"],
    process.env.ProgramW6432,
  ]
    .filter((value) => typeof value === "string" && value.trim())
    .map((value) => path.resolve(value));
}

function candidateInsideProgramFiles(candidate, roots) {
  const resolvedCandidate = path.resolve(candidate);
  return roots.some((root) => isPathInside(root, resolvedCandidate));
}

function registryApplicationCandidates(entry) {
  const candidates = [];
  const installLocation = registryDirectory(entry.values.installlocation);
  if (installLocation) {
    candidates.push(path.join(installLocation, INSTALLED_EXECUTABLE));
  }

  const displayIcon = registryExecutable(entry.values.displayicon);
  if (displayIcon) {
    candidates.push(displayIcon);
  }

  return [
    ...new Map(
      candidates.map((candidate) => [
        path.resolve(candidate).toLowerCase(),
        path.resolve(candidate),
      ]),
    ).values(),
  ];
}

async function findValidatedInstalledApplication({
  requiredVersion = null,
  expectedExecutableSize = null,
  expectedExecutableSha256 = null,
} = {}) {
  const registryEntries = await findMavroRegistryEntries();
  const roots = programFilesRoots();
  if (roots.length === 0) {
    return null;
  }

  for (const entry of registryEntries) {
    if (
      requiredVersion !== null &&
      entry.values.displayversion !== requiredVersion
    ) {
      continue;
    }

    for (const candidate of registryApplicationCandidates(entry)) {
      if (
        path.basename(candidate).toLowerCase() !==
          INSTALLED_EXECUTABLE.toLowerCase() ||
        !candidateInsideProgramFiles(candidate, roots)
      ) {
        continue;
      }

      let executable;
      try {
        executable = await inspectPeFile(candidate, {
          minBytes: MIN_INSTALLED_BYTES,
        });
      } catch {
        continue;
      }

      // realpath() can resolve junctions and symlinks. Re-check the actual PE
      // location so a registry path that only appears to be under Program Files
      // cannot enable update mode or be launched.
      if (
        path.basename(executable.path).toLowerCase() !==
          INSTALLED_EXECUTABLE.toLowerCase() ||
        !candidateInsideProgramFiles(executable.path, roots)
      ) {
        continue;
      }
      if (
        expectedExecutableSize !== null &&
        executable.size !== expectedExecutableSize
      ) {
        continue;
      }
      if (
        expectedExecutableSha256 !== null &&
        executable.sha256 !== expectedExecutableSha256
      ) {
        continue;
      }

      return {
        executablePath: executable.path,
        registryKey: entry.key,
        displayVersion: entry.values.displayversion ?? null,
        size: executable.size,
        sha256: executable.sha256,
      };
    }
  }

  return null;
}

async function verifyInstalledApplication(expected) {
  const verified = await findValidatedInstalledApplication({
    requiredVersion: PRODUCT_VERSION,
    expectedExecutableSize: expected.installedExecutableSize,
    expectedExecutableSha256: expected.installedExecutableSha256,
  });
  if (verified) {
    return {
      ...verified,
      expectedExecutableSize: expected.installedExecutableSize,
      expectedExecutableSha256: expected.installedExecutableSha256,
    };
  }

  throw new Error(
    `NSIS işlemi tamamlandı ancak Program Files altındaki MavroDPI.exe; DisplayVersion ${PRODUCT_VERSION}, beklenen boyut ve SHA256 ile birlikte doğrulanamadı.`,
  );
}

function launchInstalledApplication(executablePath) {
  const child = spawn(executablePath, [], {
    cwd: path.dirname(executablePath),
    detached: true,
    stdio: "ignore",
    windowsHide: false,
  });
  child.unref();
}

function decodePowerShellError(encoded) {
  try {
    return Buffer.from(encoded, "base64").toString("utf8").trim();
  } catch {
    return "Yönetici onayı veya kurulum süreci tamamlanamadı.";
  }
}

async function runElevatedNsis(payloadPath, updateMode) {
  const markerDirectory = await fs.promises.mkdtemp(
    path.join(os.tmpdir(), "mavrodpi-setup-"),
  );
  const statusPath = path.join(markerDirectory, "status.txt");
  const resultPath = path.join(markerDirectory, "result.txt");
  let installingEmitted = false;
  let stderr = "";

  try {
    emitStatus({
      phase: "awaiting-uac",
      message: "Windows yönetici onayı bekleniyor.",
      exitCode: null,
    });

    const child = spawn(
      systemExecutable("WindowsPowerShell\\v1.0\\powershell.exe"),
      [
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-EncodedCommand",
        POWERSHELL_COMMAND,
      ],
      {
        windowsHide: true,
        stdio: ["ignore", "ignore", "pipe"],
        env: {
          ...process.env,
          MAVRODPI_PAYLOAD_PATH: payloadPath,
          MAVRODPI_STATUS_PATH: statusPath,
          MAVRODPI_RESULT_PATH: resultPath,
          MAVRODPI_UPDATE_MODE: updateMode ? "1" : "0",
        },
      },
    );

    child.stderr.setEncoding("utf8");
    child.stderr.on("data", (chunk) => {
      if (stderr.length < 8192) {
        stderr += chunk;
      }
    });

    const markerTimer = setInterval(() => {
      if (!installingEmitted && fs.existsSync(statusPath)) {
        installingEmitted = true;
        emitStatus({
          phase: "installing",
          message: "Gerçek NSIS kurulumu çalışıyor.",
        });
      }
    }, 180);

    const processCode = await new Promise((resolve, reject) => {
      child.once("error", reject);
      child.once("close", (code) => resolve(code));
    }).finally(() => clearInterval(markerTimer));

    if (!installingEmitted && fs.existsSync(statusPath)) {
      installingEmitted = true;
      emitStatus({
        phase: "installing",
        message: "Gerçek NSIS kurulumu çalıştı.",
      });
    }

    let resultText = "";
    try {
      resultText = (await fs.promises.readFile(resultPath, "utf8")).trim();
    } catch {
      // The process exit code and stderr remain available as fallback evidence.
    }

    if (resultText.startsWith("ERROR:")) {
      const error = new Error(decodePowerShellError(resultText.slice(6)));
      error.exitCode = processCode;
      throw error;
    }

    const recordedExit = resultText.match(/^EXIT:(-?\d+)$/);
    const exitCode = recordedExit ? Number(recordedExit[1]) : processCode;
    if (!Number.isInteger(exitCode)) {
      throw new Error(
        stderr.trim() || "NSIS kurulum süreci bir çıkış kodu üretmedi.",
      );
    }

    emitStatus({
      phase: "installer-exited",
      message: `NSIS kurulum süreci ${exitCode} koduyla kapandı.`,
      exitCode,
    });

    // Non-zero NSIS status is not accepted by itself. The caller validates the
    // final Program Files executable, uninstall record, version, size, and hash.
    return { exitCode, nonZeroExit: exitCode !== 0 };
  } finally {
    await fs.promises.rm(markerDirectory, { recursive: true, force: true });
  }
}

async function performInstall({ launchAfterInstall }) {
  if (installationBusy) {
    return {
      success: false,
      message: "Kurulum zaten devam ediyor.",
    };
  }

  installationBusy = true;
  lastVerifiedInstall = null;

  try {
    emitStatus({
      phase: "verifying-payload",
      message: "Paket içindeki NSIS dosyasının boyutu ve PE imzası doğrulanıyor.",
      exitCode: null,
      installedPath: null,
      registryKey: null,
      installedVersion: null,
    });
    const payload = await inspectPackagedPayload();

    emitStatus({
      phase: "payload-verified",
      message: `Paket doğrulandı · SHA256 ${payload.sha256.slice(0, 12).toUpperCase()}`,
    });

    const existingInstall = await findValidatedInstalledApplication();
    const existingRecord = existingInstall ?? (await detectExistingMavroInstall());
    const installMode = existingRecord ? "repair" : "install";
    if (installMode === "repair") {
      emitStatus({
        phase: "installing",
        message: "Önceki MavroDPI kurulumu bulundu; onar ve güncelleme modu başlatılıyor.",
      });
    }
    const nsisResult = await runElevatedNsis(
      payload.path,
      installMode === "repair",
    );
    const exitCode = nsisResult.exitCode;

    emitStatus({
      phase: "verifying-installation",
      message: "Kurulu uygulama ve Windows kaldırma kaydı doğrulanıyor.",
      exitCode,
    });
    const verified = await verifyInstalledApplication(payload);
    lastVerifiedInstall = verified;

    if (launchAfterInstall) {
      emitStatus({
        phase: "launching",
        message: "Doğrulanan MavroDPI uygulaması başlatılıyor.",
        installedPath: verified.executablePath,
        registryKey: verified.registryKey,
      });
      launchInstalledApplication(verified.executablePath);
    }

    emitStatus({
      phase: "complete",
      message: nsisResult.nonZeroExit
        ? "NSIS sıfır dışı kod döndürdü; ancak kurulu uygulama, sürüm ve SHA-256 kimliği eksiksiz doğrulandı."
        : launchAfterInstall
          ? "Kurulum doğrulandı ve MavroDPI başlatıldı."
          : "Kurulum ve Windows kayıtları doğrulandı.",
      exitCode,
      installedPath: verified.executablePath,
      registryKey: verified.registryKey,
      installedVersion: verified.displayVersion,
      launched: launchAfterInstall,
      installMode,
    });

    return {
      success: true,
      exitCode,
      installedPath: verified.executablePath,
      registryKey: verified.registryKey,
      installedVersion: verified.displayVersion,
      launched: launchAfterInstall,
      installMode,
    };
  } catch (error) {
    const message = safeErrorMessage(
      error,
      "Kurulum güvenli biçimde tamamlanamadı.",
    );
    emitStatus({
      phase: "failed",
      message,
      exitCode: Number.isInteger(error?.exitCode) ? error.exitCode : null,
    });
    return {
      success: false,
      message,
      exitCode: Number.isInteger(error?.exitCode) ? error.exitCode : null,
    };
  } finally {
    installationBusy = false;
  }
}

async function getInstallerInfo() {
  try {
    const payload = await inspectPackagedPayload();
    const existingInstall = await detectExistingMavroInstall();
    return {
      productName: PRODUCT_NAME,
      version: PRODUCT_VERSION,
      architecture: "x64",
      payloadReady: true,
      payloadBytes: payload.size,
      payloadHash: payload.sha256,
      payloadVersion: payload.version,
      installMode: existingInstall ? "repair" : "install",
      installedVersion: existingInstall?.displayVersion ?? null,
      status: currentStatus,
    };
  } catch (error) {
    return {
      productName: PRODUCT_NAME,
      version: PRODUCT_VERSION,
      architecture: "x64",
      payloadReady: false,
      payloadBytes: null,
      payloadHash: null,
      payloadError: safeErrorMessage(
        error,
        "Kurulum payload'ı bu yapıya eklenmemiş.",
      ),
      status: currentStatus,
    };
  }
}

function assertTrustedSender(event) {
  if (
    !mainWindow ||
    mainWindow.isDestroyed() ||
    event.sender !== mainWindow.webContents
  ) {
    throw new Error("Güvenilmeyen IPC kaynağı reddedildi.");
  }
}

function registerIpc() {
  ipcMain.handle("installer:get-info", async (event) => {
    assertTrustedSender(event);
    return getInstallerInfo();
  });

  ipcMain.handle("installer:install", async (event, options) => {
    assertTrustedSender(event);
    return performInstall({
      launchAfterInstall: options?.launchAfterInstall === true,
    });
  });

  ipcMain.handle("installer:launch", async (event) => {
    assertTrustedSender(event);
    if (installationBusy || !lastVerifiedInstall) {
      return { success: false, message: "Doğrulanmış kurulum bulunamadı." };
    }

    const verified = await verifyInstalledApplication({
      installedExecutableSize: lastVerifiedInstall.expectedExecutableSize,
      installedExecutableSha256:
        lastVerifiedInstall.expectedExecutableSha256,
    });
    lastVerifiedInstall = verified;
    launchInstalledApplication(verified.executablePath);
    return { success: true, installedPath: verified.executablePath };
  });

  ipcMain.handle("window:close", (event) => {
    assertTrustedSender(event);
    if (installationBusy) {
      return { closed: false, reason: "Kurulum devam ediyor." };
    }

    allowWindowClose = true;
    mainWindow.close();
    return { closed: true };
  });
}

function hardenWebContents(contents) {
  contents.setWindowOpenHandler(() => ({ action: "deny" }));
  contents.on("will-navigate", (event) => event.preventDefault());
  contents.on("will-attach-webview", (event) => event.preventDefault());
}

function createWindow() {
  mainWindow = new BrowserWindow({
    width: 940,
    height: 680,
    minWidth: 940,
    minHeight: 680,
    maxWidth: 940,
    maxHeight: 680,
    show: false,
    frame: false,
    transparent: true,
    backgroundColor: "#00000000",
    resizable: false,
    minimizable: false,
    maximizable: false,
    fullscreenable: false,
    autoHideMenuBar: true,
    title: "MavroDPI Setup",
    webPreferences: {
      preload: path.join(__dirname, "preload.cjs"),
      contextIsolation: true,
      sandbox: true,
      nodeIntegration: false,
      webviewTag: false,
      webSecurity: true,
      allowRunningInsecureContent: false,
      devTools: false,
      spellcheck: false,
    },
  });

  mainWindow.setMenu(null);
  hardenWebContents(mainWindow.webContents);
  mainWindow.loadFile(path.join(__dirname, "renderer", "index.html"));
  mainWindow.once("ready-to-show", () => mainWindow.show());
  mainWindow.on("close", (event) => {
    if (installationBusy && !allowWindowClose) {
      event.preventDefault();
    }
  });
  mainWindow.on("closed", () => {
    mainWindow = null;
  });
}

if (!app.requestSingleInstanceLock()) {
  app.quit();
} else {
  app.on("second-instance", () => {
    if (mainWindow) {
      mainWindow.show();
      mainWindow.focus();
    }
  });

  app.whenReady().then(() => {
    session.defaultSession.setPermissionRequestHandler(
      (_webContents, _permission, callback) => callback(false),
    );
    session.defaultSession.setPermissionCheckHandler(() => false);
    app.on("web-contents-created", (_event, contents) =>
      hardenWebContents(contents),
    );

    registerIpc();
    createWindow();
  });
}

app.on("window-all-closed", () => {
  if (!installationBusy) {
    app.quit();
  }
});
