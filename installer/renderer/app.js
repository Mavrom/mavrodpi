"use strict";

const elements = {
  packageCard: document.querySelector("#packageCard"),
  packageDetail: document.querySelector("#packageDetail"),
  packageState: document.querySelector("#packageState"),
  statusTitle: document.querySelector("#statusTitle"),
  statusDetail: document.querySelector("#statusDetail"),
  statusBeacon: document.querySelector("#statusBeacon"),
  installedPath: document.querySelector("#installedPath"),
  exitCode: document.querySelector("#exitCode"),
  launchAfter: document.querySelector("#launchAfter"),
  installButton: document.querySelector("#installButton"),
  installButtonLabel: document.querySelector("#installButton span"),
  closeButton: document.querySelector("#closeButton"),
  launchChoice: document.querySelector(".launch-choice"),
  steps: {
    payload: document.querySelector('[data-step="payload"]'),
    uac: document.querySelector('[data-step="uac"]'),
    install: document.querySelector('[data-step="install"]'),
    verify: document.querySelector('[data-step="verify"]'),
  },
};

const phaseCopy = {
  ready: ["Kuruluma hazır", "Kurulum başlatılmayı bekliyor."],
  "verifying-payload": [
    "Paket doğrulanıyor",
    "Paket boyutu ile MZ ve PE imzası kontrol ediliyor.",
  ],
  "payload-verified": [
    "Paket doğrulandı",
    "Yalnız paket içindeki doğrulanmış NSIS dosyası kullanılacak.",
  ],
  "awaiting-uac": [
    "Yönetici onayı bekleniyor",
    "Devam etmek için Windows UAC ekranını onayla.",
  ],
  installing: [
    "Kurulum çalışıyor",
    "Gerçek NSIS işlemi sessiz kurulum modunda çalışıyor.",
  ],
  "installer-exited": [
    "Kurulum süreci kapandı",
    "NSIS çıkış kodu alındı; sistem doğrulamasına geçiliyor.",
  ],
  "verifying-installation": [
    "Sistem doğrulanıyor",
    "Kurulu MavroDPI.exe ve Windows kaldırma kaydı aranıyor.",
  ],
  launching: [
    "MavroDPI başlatılıyor",
    "Yalnız doğrulanmış kurulu uygulama açılıyor.",
  ],
  complete: ["Kurulum tamamlandı", "Uygulama ve Windows kayıtları doğrulandı."],
  failed: ["Kurulum tamamlanamadı", "Ayrıntıyı kontrol edip yeniden deneyebilirsin."],
};

let payloadReady = false;
let currentPhase = "ready";
let installRunning = false;
let installationComplete = false;
let launchedAfterInstall = false;

function setStep(step, state) {
  const item = elements.steps[step];
  item.classList.remove("is-active", "is-done", "is-error");
  if (state !== "pending") {
    item.classList.add(`is-${state}`);
  }

  const label = item.querySelector(".step-state");
  label.textContent =
    state === "active"
      ? "ÇALIŞIYOR"
      : state === "done"
        ? "TAMAM"
        : state === "error"
          ? "HATA"
          : "BEKLİYOR";
}

function resetSteps() {
  for (const step of Object.keys(elements.steps)) {
    setStep(step, "pending");
  }
  elements.exitCode.textContent = "Gerçek işlem çıkışı";
}

function applyPhaseToSteps(phase) {
  resetSteps();

  if (phase === "verifying-payload") {
    setStep("payload", "active");
  } else if (phase === "payload-verified" || phase === "awaiting-uac") {
    setStep("payload", "done");
    setStep("uac", "active");
  } else if (phase === "installing") {
    setStep("payload", "done");
    setStep("uac", "done");
    setStep("install", "active");
  } else if (
    phase === "installer-exited" ||
    phase === "verifying-installation"
  ) {
    setStep("payload", "done");
    setStep("uac", "done");
    setStep("install", "done");
    setStep("verify", "active");
  } else if (phase === "launching" || phase === "complete") {
    for (const step of Object.keys(elements.steps)) {
      setStep(step, "done");
    }
  } else if (phase === "failed") {
    const failedStep =
      currentPhase === "verifying-payload"
        ? "payload"
        : currentPhase === "awaiting-uac"
          ? "uac"
          : currentPhase === "installing" ||
              currentPhase === "installer-exited"
            ? "install"
            : "verify";

    const order = ["payload", "uac", "install", "verify"];
    for (const step of order) {
      if (step === failedStep) {
        setStep(step, "error");
        break;
      }
      setStep(step, "done");
    }
  }
}

function setBusy(busy) {
  installRunning = busy;
  elements.installButton.disabled = busy || (!payloadReady && !installationComplete);
  elements.closeButton.disabled = busy;
  elements.launchAfter.disabled = busy || installationComplete;
  elements.launchChoice.classList.toggle("is-disabled", busy || installationComplete);
  if (busy) {
    elements.installButtonLabel.textContent = "İşlem sürüyor";
  }
}

function renderStatus(status) {
  if (!status || typeof status.phase !== "string") {
    return;
  }

  const previousPhase = currentPhase;
  currentPhase = status.phase;
  const copy = phaseCopy[status.phase] || phaseCopy.ready;
  elements.statusTitle.textContent = copy[0];
  elements.statusDetail.textContent =
    typeof status.message === "string" && status.message.trim()
      ? status.message
      : copy[1];

  if (Number.isInteger(status.exitCode)) {
    elements.exitCode.textContent = `Exit code: ${status.exitCode}`;
  }

  if (typeof status.installedPath === "string" && status.installedPath) {
    elements.installedPath.hidden = false;
    elements.installedPath.textContent = status.installedPath;
  } else if (status.phase !== "complete") {
    elements.installedPath.hidden = true;
    elements.installedPath.textContent = "";
  }

  if (status.phase === "failed") {
    currentPhase = previousPhase;
    applyPhaseToSteps("failed");
    currentPhase = "failed";
  } else {
    applyPhaseToSteps(status.phase);
  }

  const active = !["ready", "complete", "failed"].includes(status.phase);
  elements.statusBeacon.classList.toggle("is-active", active);
  elements.statusBeacon.classList.toggle("is-done", status.phase === "complete");

  if (status.phase === "complete") {
    installationComplete = true;
    launchedAfterInstall = status.launched === true;
    setBusy(false);
    elements.closeButton.textContent = "Kapat";
    elements.installButton.disabled = false;
    elements.installButtonLabel.textContent = launchedAfterInstall
      ? "Kapat"
      : "MavroDPI’yi aç";
  } else if (status.phase === "failed") {
    setBusy(false);
    elements.closeButton.textContent = "Kapat";
    elements.installButtonLabel.textContent = payloadReady
      ? "Yeniden dene"
      : "Kullanılamıyor";
    elements.installButton.disabled = !payloadReady;
  } else if (active) {
    setBusy(true);
  }
}

function formatBytes(bytes) {
  if (!Number.isFinite(bytes)) {
    return "";
  }
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

async function loadInfo() {
  try {
    const info = await window.mavroInstaller.getInfo();
    payloadReady = info.payloadReady === true;
    elements.packageCard.classList.toggle("is-ready", payloadReady);
    elements.packageCard.classList.toggle("is-error", !payloadReady);
    elements.packageState.textContent = payloadReady ? "DOĞRULANDI" : "EKSİK";
    elements.packageDetail.textContent = payloadReady
      ? `${formatBytes(info.payloadBytes)} · SHA256 ${info.payloadHash
          .slice(0, 12)
          .toUpperCase()}`
      : info.payloadError || "Payload bu yapıya eklenmedi.";
    elements.installButton.disabled = !payloadReady;
    renderStatus(info.status);
  } catch {
    payloadReady = false;
    elements.packageCard.classList.add("is-error");
    elements.packageState.textContent = "HATA";
    elements.packageDetail.textContent = "Paket bilgisi alınamadı.";
    elements.installButton.disabled = true;
  }
}

window.mavroInstaller.onStatus(renderStatus);

elements.installButton.addEventListener("click", async () => {
  if (installationComplete) {
    if (launchedAfterInstall) {
      await window.mavroInstaller.close();
      return;
    }

    elements.installButton.disabled = true;
    elements.installButtonLabel.textContent = "Başlatılıyor";
    const result = await window.mavroInstaller.launch();
    if (result?.success) {
      launchedAfterInstall = true;
      elements.installButtonLabel.textContent = "Kapat";
      elements.installButton.disabled = false;
      elements.statusDetail.textContent = "Doğrulanan MavroDPI uygulaması başlatıldı.";
    } else {
      elements.installButtonLabel.textContent = "Tekrar dene";
      elements.installButton.disabled = false;
      elements.statusDetail.textContent =
        result?.message || "MavroDPI başlatılamadı.";
    }
    return;
  }

  if (!payloadReady || installRunning) {
    return;
  }

  setBusy(true);
  const result = await window.mavroInstaller.install(
    elements.launchAfter.checked,
  );
  if (!result?.success && currentPhase !== "failed") {
    renderStatus({
      phase: "failed",
      message: result?.message || "Kurulum tamamlanamadı.",
      exitCode: result?.exitCode,
    });
  }
});

elements.closeButton.addEventListener("click", () => {
  if (!installRunning) {
    window.mavroInstaller.close();
  }
});

loadInfo();
