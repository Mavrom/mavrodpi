import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
} from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import {
  DEFAULT_PROFILE_ID,
  PROTECTION_PROFILES,
  getProtectionProfile,
  type ProtectionProfileId,
} from "./profiles";
import "./App.css";

interface NetSample {
  rx: number;
  tx: number;
}

interface SiteStatus {
  name: string;
  host: string;
  reachable: boolean;
  latencyMs: number | null;
  statusCode: number | null;
  detail: string;
}

interface EngineInfo {
  running: boolean;
  profile: ProtectionProfileId | null;
  pid: number | null;
  startedAt: number | null;
}

interface ServiceStatus {
  installed: boolean;
  running: boolean;
  state: string;
  binaryPathCurrent: boolean;
  helperHashCurrent: boolean;
  payloadCurrent: boolean;
  needsRepair: boolean;
}

type ContextTab = "diagnostics" | "settings";

const PROFILE_STORAGE_KEY = "mavrodpi-profile";
const MOTION_STORAGE_KEY = "mavrodpi-reduce-motion";

function readStoredProfile(): ProtectionProfileId {
  const value = localStorage.getItem(PROFILE_STORAGE_KEY);
  return value === "compatibility" || value === "balanced"
    ? value
    : DEFAULT_PROFILE_ID;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${Math.round(bytes)} B`;
  if (bytes < 1024 ** 2) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 ** 3) return `${(bytes / 1024 ** 2).toFixed(1)} MB`;
  return `${(bytes / 1024 ** 3).toFixed(2)} GB`;
}

function formatRate(bytes: number): string {
  return `${formatBytes(bytes)}/sn`;
}

function formatDuration(seconds: number): string {
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const remainingSeconds = Math.floor(seconds % 60);
  return [hours, minutes, remainingSeconds]
    .map((value) => String(value).padStart(2, "0"))
    .join(":");
}

function currentTime(): string {
  return new Intl.DateTimeFormat("tr-TR", {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  }).format(new Date());
}

function serviceStateLabel(state: string): string {
  const labels: Record<string, string> = {
    running: "ÇALIŞIYOR",
    stopped: "DURDU",
    start_pending: "BAŞLATILIYOR",
    stop_pending: "DURDURULUYOR",
    paused: "DURAKLATILDI",
    pause_pending: "DURAKLATILIYOR",
    continue_pending: "SÜRDÜRÜLÜYOR",
    not_installed: "KURULU DEĞİL",
    unknown: "BİLİNMİYOR",
  };
  return labels[state] ?? "BİLİNMİYOR";
}

export default function App() {
  const [running, setRunning] = useState(false);
  const [busy, setBusy] = useState(false);
  const [isAdmin, setIsAdmin] = useState(false);
  const [adminRelaunching, setAdminRelaunching] = useState(false);
  const [dohOn, setDohOn] = useState(false);
  const [dohManaged, setDohManaged] = useState(false);
  const [resettingExternalDns, setResettingExternalDns] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [engineInfo, setEngineInfo] = useState<EngineInfo>({
    running: false,
    profile: null,
    pid: null,
    startedAt: null,
  });

  const [serviceStatus, setServiceStatus] = useState<ServiceStatus>({
    installed: false,
    running: false,
    state: "not_installed",
    binaryPathCurrent: false,
    helperHashCurrent: false,
    payloadCurrent: false,
    needsRepair: false,
  });
  const [serviceReady, setServiceReady] = useState(false);
  const [serviceQueryError, setServiceQueryError] = useState<string | null>(
    null,
  );
  const [serviceBusy, setServiceBusy] = useState(false);
  const [maximized, setMaximized] = useState(false);

  const [selectedProfile, setSelectedProfile] =
    useState<ProtectionProfileId>(readStoredProfile);
  const [reduceMotion, setReduceMotion] = useState(
    () => localStorage.getItem(MOTION_STORAGE_KEY) === "true",
  );
  const [activeTab, setActiveTab] = useState<ContextTab>("diagnostics");

  const [updateAvailable, setUpdateAvailable] = useState(false);
  const [updateVersion, setUpdateVersion] = useState("");
  const [updating, setUpdating] = useState(false);

  const [uptime, setUptime] = useState(0);
  const [rxRate, setRxRate] = useState(0);
  const [txRate, setTxRate] = useState(0);
  const [sessionRx, setSessionRx] = useState(0);
  const [sessionTx, setSessionTx] = useState(0);

  const [sites, setSites] = useState<SiteStatus[]>([]);
  const [checking, setChecking] = useState(false);
  const [hasChecked, setHasChecked] = useState(false);

  const [logs, setLogs] = useState<string[]>([]);

  const startedAtRef = useRef(0);
  const previousNetRef = useRef<{
    rx: number;
    tx: number;
    measuredAt: number;
  } | null>(null);
  const logEndRef = useRef<HTMLDivElement>(null);

  const profile = getProtectionProfile(selectedProfile);
  const serviceOn = serviceStatus.installed;
  const serviceRunning = serviceStatus.running;
  const serviceLocksForeground = serviceOn && !running;
  const stateLabel = !serviceReady
    ? "SERVİS DURUMU BELİRSİZ"
    : running
    ? "YEREL DPI AÇIK"
    : serviceStatus.needsRepair
      ? "SERVİS ONARIM GEREKLİ"
      : serviceRunning
      ? "SERVİS ÇALIŞIYOR"
      : serviceOn
        ? "SERVİS DURDU"
      : "HAZIR";

  const addLog = useCallback((message: string) => {
    setLogs((previous) => [
      ...previous.slice(-199),
      `${currentTime()}  ${message}`,
    ]);
  }, []);

  const syncEngineInfo = useCallback(async () => {
    const info = await invoke<EngineInfo>("get_engine_info");
    setEngineInfo(info);
    setRunning(info.running);
    if (
      info.profile === "balanced" ||
      info.profile === "compatibility"
    ) {
      setSelectedProfile(info.profile);
      localStorage.setItem(PROFILE_STORAGE_KEY, info.profile);
    }
    return info;
  }, []);

  const syncServiceStatus = useCallback(async () => {
    try {
      const status = await invoke<ServiceStatus>("service_status");
      setServiceStatus(status);
      setServiceReady(true);
      setServiceQueryError(null);
      return status;
    } catch (caughtError) {
      setServiceReady(false);
      setServiceQueryError(
        `Windows servis durumu okunamadı: ${String(caughtError)}`,
      );
      throw caughtError;
    }
  }, []);

  useEffect(() => {
    document.body.classList.toggle("reduce-motion", reduceMotion);
    localStorage.setItem(MOTION_STORAGE_KEY, String(reduceMotion));
  }, [reduceMotion]);

  useEffect(() => {
    const initialize = async () => {
      const [status, engine, admin, doh, managedDoh, service] =
        await Promise.allSettled([
        invoke<boolean>("get_status"),
        invoke<EngineInfo>("get_engine_info"),
        invoke<boolean>("is_admin"),
        invoke<boolean>("doh_status"),
        invoke<boolean>("doh_managed"),
        invoke<ServiceStatus>("service_status"),
      ]);

      if (status.status === "fulfilled") setRunning(status.value);
      if (engine.status === "fulfilled") {
        setEngineInfo(engine.value);
        setRunning(engine.value.running);
        if (
          engine.value.profile === "balanced" ||
          engine.value.profile === "compatibility"
        ) {
          setSelectedProfile(engine.value.profile);
          localStorage.setItem(PROFILE_STORAGE_KEY, engine.value.profile);
        }
      }
      if (admin.status === "fulfilled") setIsAdmin(admin.value);
      if (doh.status === "fulfilled") setDohOn(doh.value);
      if (managedDoh.status === "fulfilled") {
        setDohManaged(managedDoh.value);
      }
      if (service.status === "fulfilled") {
        setServiceStatus(service.value);
        setServiceReady(true);
        setServiceQueryError(null);
      } else {
        setServiceReady(false);
        setServiceQueryError(
          `Windows servis durumu okunamadı: ${String(service.reason)}`,
        );
      }
    };

    void initialize();
    void getCurrentWindow()
      .isMaximized()
      .then(setMaximized)
      .catch(() => {});

    void check()
      .then((update) => {
        if (update?.available) {
          setUpdateAvailable(true);
          setUpdateVersion(update.version);
        }
      })
      .catch(() => {});

    const removeStatusListener = listen<boolean>("gdpi-status", (event) => {
      setRunning(event.payload);
      if (event.payload) {
        void syncEngineInfo().catch(() => {});
      } else {
        setEngineInfo({
          running: false,
          profile: null,
          pid: null,
          startedAt: null,
        });
      }
    });
    const removeLogListener = listen<string>("gdpi-log", (event) => {
      addLog(event.payload);
    });
    const removeDohStatusListener = listen<boolean>("doh-status", (event) => {
      setDohOn(event.payload);
      void invoke<boolean>("doh_managed")
        .then(setDohManaged)
        .catch(() => setDohManaged(false));
    });

    return () => {
      void removeStatusListener.then((remove) => remove());
      void removeLogListener.then((remove) => remove());
      void removeDohStatusListener.then((remove) => remove());
    };
  }, [addLog, syncEngineInfo]);

  useEffect(() => {
    const interval = window.setInterval(
      () => void syncServiceStatus().catch(() => {}),
      5000,
    );
    return () => window.clearInterval(interval);
  }, [syncServiceStatus]);

  useEffect(() => {
    if (!running) {
      setUptime(0);
      setRxRate(0);
      setTxRate(0);
      setSessionRx(0);
      setSessionTx(0);
      previousNetRef.current = null;
      return;
    }

    startedAtRef.current = engineInfo.startedAt
      ? engineInfo.startedAt * 1000
      : Date.now();
    previousNetRef.current = null;

    const sampleNetwork = async () => {
      setUptime((Date.now() - startedAtRef.current) / 1000);
      try {
        const sample = await invoke<NetSample>("net_stats");
        const measuredAt = Date.now();
        const previous = previousNetRef.current;

        if (previous) {
          const seconds = (measuredAt - previous.measuredAt) / 1000;
          const received = Math.max(0, sample.rx - previous.rx);
          const transmitted = Math.max(0, sample.tx - previous.tx);
          if (seconds > 0) {
            setRxRate(received / seconds);
            setTxRate(transmitted / seconds);
            setSessionRx((value) => value + received);
            setSessionTx((value) => value + transmitted);
          }
        }

        previousNetRef.current = {
          rx: sample.rx,
          tx: sample.tx,
          measuredAt,
        };
      } catch {
        // Ağ sayacı okunamazsa koruma çalışmaya devam eder.
      }
    };

    void sampleNetwork();
    const interval = window.setInterval(() => void sampleNetwork(), 1000);
    return () => window.clearInterval(interval);
  }, [engineInfo.startedAt, running]);

  useEffect(() => {
    logEndRef.current?.scrollIntoView({
      behavior: reduceMotion ? "auto" : "smooth",
      block: "nearest",
    });
  }, [logs, reduceMotion]);

  const startProtection = useCallback(async () => {
    if (
      !isAdmin ||
      !serviceReady ||
      serviceLocksForeground ||
      serviceBusy ||
      updating
    )
      return;

    setBusy(true);
    setError(null);
    let enabledDohForThisStart = false;
    let dohWarning: string | null = null;

    try {
      if (!dohOn) {
        try {
          await invoke("enable_doh");
          setDohOn(true);
          setDohManaged(true);
          enabledDohForThisStart = true;
          addLog("Windows şifreli DNS ayarı etkinleştirildi.");
        } catch (caughtError) {
          dohWarning = String(caughtError);
          const detectedDoh = await invoke<boolean>("doh_status").catch(
            () => false,
          );
          const detectedManaged = await invoke<boolean>("doh_managed").catch(
            () => false,
          );
          setDohOn(detectedDoh);
          setDohManaged(detectedManaged);
          enabledDohForThisStart = detectedDoh && detectedManaged;
          addLog(
            `Şifreli DNS etkinleştirilemedi; DPI motoru tek başına başlatılıyor: ${dohWarning}`,
          );
        }
      }

      await invoke("start_protection", { args: [...profile.args] });
      setRunning(true);
      await syncEngineInfo().catch(() => {
        setEngineInfo({
          running: true,
          profile: profile.id,
          pid: null,
          startedAt: Math.floor(Date.now() / 1000),
        });
      });
      addLog(
        `${profile.name} profili ${profile.args.join(" ")} argümanıyla başlatıldı.`,
      );
      if (dohWarning) {
        setError(
          `DPI motoru çalışıyor; ancak şifreli DNS etkinleştirilemedi. DNS tabanlı engeller devam edebilir: ${dohWarning}`,
        );
      }
    } catch (caughtError) {
      let dnsRollbackError: string | null = null;
      if (enabledDohForThisStart) {
        try {
          await invoke("disable_doh");
        } catch (rollbackError) {
          dnsRollbackError = String(rollbackError);
          addLog(
            `[hata] Motor başlatılamadıktan sonra DNS geri alınamadı: ${dnsRollbackError}`,
          );
        }
        const detectedDoh = await invoke<boolean>("doh_status").catch(
          () => false,
        );
        const detectedManaged = await invoke<boolean>("doh_managed").catch(
          () => false,
        );
        setDohOn(detectedDoh);
        setDohManaged(detectedManaged);
      }
      setError(
        dnsRollbackError
          ? `${String(caughtError)} DNS geri alma işlemi de tamamlanamadı: ${dnsRollbackError}`
          : String(caughtError),
      );
    } finally {
      setBusy(false);
    }
  }, [
    addLog,
    dohOn,
    isAdmin,
    profile,
    serviceReady,
    serviceBusy,
    serviceLocksForeground,
    syncEngineInfo,
    updating,
  ]);

  const stopProtection = useCallback(async () => {
    setBusy(true);
    setError(null);

    try {
      await invoke("stop_protection");
      setRunning(false);
      setEngineInfo({
        running: false,
        profile: null,
        pid: null,
        startedAt: null,
      });
      const detectedDoh = await invoke<boolean>("doh_status").catch(
        () => false,
      );
      const detectedManaged = await invoke<boolean>("doh_managed").catch(
        () => false,
      );
      setDohOn(detectedDoh);
      setDohManaged(detectedManaged);
      addLog("Yerel DPI süreci durduruldu ve önceki Windows DNS ayarı geri yüklendi.");
    } catch (caughtError) {
      setError(String(caughtError));
      void syncEngineInfo().catch(() => {
        void invoke<boolean>("get_status").then(setRunning).catch(() => {});
      });
      void invoke<boolean>("doh_status").then(setDohOn).catch(() => {});
      void invoke<boolean>("doh_managed")
        .then(setDohManaged)
        .catch(() => setDohManaged(false));
    } finally {
      setBusy(false);
    }
  }, [addLog, syncEngineInfo]);

  function chooseProfile(id: ProtectionProfileId) {
    if (running || serviceOn || busy || serviceBusy || updating) return;
    const nextProfile = getProtectionProfile(id);
    setSelectedProfile(id);
    localStorage.setItem(PROFILE_STORAGE_KEY, id);
    addLog(
      `${nextProfile.name} seçildi; sonraki başlangıç ${nextProfile.args.join(" ")} argümanını kullanacak.`,
    );
  }

  async function runDiagnostics() {
    setChecking(true);
    setHasChecked(false);
    setSites([]);
    setError(null);

    try {
      const result = await invoke<SiteStatus[]>("check_sites");
      setSites(result);
      setHasChecked(true);
      const reachable = result.filter((site) => site.reachable).length;
      addLog(
        `Gerçek bağlantı kontrolü tamamlandı: ${reachable}/${result.length} hedef erişilebilir.`,
      );
    } catch (caughtError) {
      setError(String(caughtError));
      addLog(`Bağlantı kontrolü başarısız: ${String(caughtError)}`);
    } finally {
      setChecking(false);
    }
  }

  async function handleService() {
    if (updating) return;
    setServiceBusy(true);
    setError(null);

    try {
      if (serviceOn && serviceStatus.needsRepair) {
        const previousState = serviceStatus.state;
        const wasRunning = serviceStatus.running;
        await invoke("install_service");
        const status = await syncServiceStatus();
        if (
          status.needsRepair ||
          status.running !== wasRunning ||
          status.state !== previousState
        ) {
          throw new Error(
            "Servis güncellendi ancak önceki çalışma tercihi veya bütünlük durumu doğrulanamadı.",
          );
        }
        addLog(
          wasRunning
            ? "MavroDPI Windows servisi onarıldı; çalışan durum korundu."
            : previousState === "paused"
              ? "MavroDPI Windows servisi onarıldı; duraklatılmış durum korundu."
              : "MavroDPI Windows servisi onarıldı; durdurulmuş tercih korundu.",
        );
      } else if (serviceOn && serviceRunning) {
        await invoke("uninstall_service");
        await syncServiceStatus();
        addLog("MavroDPI Windows otomatik başlatma servisi kaldırıldı.");
      } else if (serviceOn) {
        let repaired = false;
        try {
          await invoke("start_service_now");
        } catch {
          await invoke("install_service");
          await invoke("start_service_now");
          repaired = true;
        }
        const status = await syncServiceStatus();
        if (!status.running) {
          throw new Error("Servis onarıldı ancak çalışma durumu doğrulanamadı.");
        }
        addLog(
          repaired
            ? "MavroDPI Windows servisi Dengeli (-5) profiliyle onarıldı ve çalışıyor."
            : "MavroDPI Windows servisi Dengeli (-5) profiliyle başlatıldı.",
        );
      } else {
        if (running) {
          await invoke("stop_protection");
          setRunning(false);
          setEngineInfo({
            running: false,
            profile: null,
            pid: null,
            startedAt: null,
          });
        }
        await invoke("install_service");
        const status = await syncServiceStatus();
        addLog(
          status.running
            ? "MavroDPI Windows servisi Dengeli (-5) profiliyle kuruldu ve çalışıyor."
            : "MavroDPI Windows servisi kuruldu ancak çalışma durumu doğrulanamadı.",
        );
      }
    } catch (caughtError) {
      setError(String(caughtError));
      void syncServiceStatus().catch(() => {});
    } finally {
      setServiceBusy(false);
    }
  }

  async function removeService() {
    if (updating) return;
    setServiceBusy(true);
    setError(null);

    try {
      await invoke("uninstall_service");
      await syncServiceStatus();
      addLog("MavroDPI Windows otomatik başlatma servisi kaldırıldı.");
    } catch (caughtError) {
      setError(String(caughtError));
      void syncServiceStatus().catch(() => {});
    } finally {
      setServiceBusy(false);
    }
  }

  async function installUpdate() {
    if (updating || busy || serviceBusy) return;

    setUpdating(true);
    setError(null);
    let phase: "checking" | "cleanup" | "installing" = "checking";

    try {
      const update = await check();
      if (!update?.available) {
        setUpdateAvailable(false);
        return;
      }

      phase = "cleanup";
      addLog(
        "Güncelleme öncesi yerel DPI süreci durduruluyor ve uygulamanın yönettiği DNS ayarı geri yükleniyor.",
      );

      // stop_protection owns the foreground child and also restores the
      // persisted DNS snapshot. The explicit disable_doh call keeps this
      // lifecycle safe when no foreground child is currently registered.
      await invoke("stop_protection");
      const foregroundStillRunning = await invoke<boolean>("get_status");
      if (foregroundStillRunning) {
        throw new Error(
          "Yerel DPI süreci durdurulduktan sonra çalışmaya devam ediyor.",
        );
      }
      await invoke("disable_doh");

      setRunning(false);
      setEngineInfo({
        running: false,
        profile: null,
        pid: null,
        startedAt: null,
      });
      const detectedDoh = await invoke<boolean>("doh_status").catch(
        () => dohOn,
      );
      const detectedManaged = await invoke<boolean>("doh_managed").catch(
        () => false,
      );
      setDohOn(detectedDoh);
      setDohManaged(detectedManaged);
      addLog(
        "Güncelleme için foreground koruma kapatıldı ve uygulamanın DNS geri yükleme işlemi tamamlandı.",
      );

      phase = "installing";
      await update.downloadAndInstall();
      await relaunch();
    } catch (caughtError) {
      const reason =
        caughtError instanceof Error
          ? caughtError.message
          : String(caughtError);
      const message =
        phase === "cleanup"
          ? `Güncelleme başlatılmadı: yerel DPI süreci veya uygulamanın yönettiği DNS ayarı güvenli biçimde kapatılamadı. ${reason}`
          : phase === "checking"
            ? `Güncelleme denetlenemedi: ${reason}`
            : `Güncelleme tamamlanamadı: ${reason}`;
      setError(message);
      addLog(`[hata] ${message}`);
    } finally {
      setUpdating(false);
    }
  }

  async function requestAdminRelaunch() {
    if (updating) return;
    setAdminRelaunching(true);
    setError(null);
    try {
      const started = await invoke<boolean>("relaunch_admin");
      if (!started) {
        setError("Windows yönetici onayı başlatılamadı veya iptal edildi.");
      }
    } catch (caughtError) {
      setError(String(caughtError));
    } finally {
      setAdminRelaunching(false);
    }
  }

  async function resetExternalDns() {
    if (
      resettingExternalDns ||
      updating ||
      busy ||
      serviceBusy ||
      !isAdmin ||
      dohManaged
    ) {
      return;
    }

    setResettingExternalDns(true);
    setError(null);
    try {
      await invoke("reset_unmanaged_dns");
      const detectedDoh = await invoke<boolean>("doh_status");
      if (detectedDoh) {
        throw new Error("Windows DNS sıfırlama işlemi doğrulanamadı.");
      }
      setDohOn(false);
      setDohManaged(false);
      addLog(
        "Harici/önceki DNS ayarı kullanıcı isteğiyle Windows otomatik DNS yapılandırmasına sıfırlandı.",
      );
    } catch (caughtError) {
      const message = `Harici DNS ayarı sıfırlanamadı: ${String(caughtError)}`;
      setError(message);
      addLog(`[hata] ${message}`);
    } finally {
      setResettingExternalDns(false);
    }
  }

  async function handleTitlebarMouseDown(
    event: ReactMouseEvent<HTMLElement>,
  ) {
    if (
      event.button !== 0 ||
      (event.target as HTMLElement).closest("button")
    ) {
      return;
    }

    try {
      const currentWindow = getCurrentWindow();
      if (event.detail === 2) {
        await currentWindow.toggleMaximize();
        setMaximized(await currentWindow.isMaximized());
      } else {
        await currentWindow.startDragging();
      }
    } catch {
      // Pencere API'si yalnızca Tauri çalışma zamanında kullanılabilir.
    }
  }

  async function handleWindowAction(
    action: "minimize" | "maximize" | "close",
  ) {
    try {
      const currentWindow = getCurrentWindow();
      if (action === "minimize") await currentWindow.minimize();
      if (action === "close") await currentWindow.close();
      if (action === "maximize") {
        await currentWindow.toggleMaximize();
        setMaximized(await currentWindow.isMaximized());
      }
    } catch (caughtError) {
      setError(String(caughtError));
    }
  }

  const reachableSiteCount = sites.filter((site) => site.reachable).length;

  return (
    <div className="window-shell">
      <header
        className="titlebar"
        onMouseDown={handleTitlebarMouseDown}
      >
        <div className="brand">
          <span className="brand-mark" aria-hidden="true">
            <i />
          </span>
          <strong>
            MAVRO<span>DPI</span>
          </strong>
          <small>/ CONTROL</small>
        </div>

        <div className={`system-state ${running ? "is-live" : ""}`}>
          <i className="state-dot" />
          <span>WINDOWS · YEREL İŞLEME · {stateLabel}</span>
        </div>

        <div className="title-actions">
          <span className={isAdmin ? "status-ready" : "status-warning"}>
            {isAdmin ? "YÖNETİCİ HAZIR" : "YETKİ GEREKLİ"}
          </span>
          <button
            type="button"
            onClick={() => handleWindowAction("minimize")}
            aria-label="Simge durumuna küçült"
            title="Simge durumuna küçült"
          >
            —
          </button>
          <button
            type="button"
            onClick={() => handleWindowAction("maximize")}
            aria-label={maximized ? "Pencereyi geri yükle" : "Pencereyi büyüt"}
            title={maximized ? "Pencereyi geri yükle" : "Pencereyi büyüt"}
          >
            {maximized ? "❐" : "□"}
          </button>
          <button
            type="button"
            className="close-button"
            onClick={() => handleWindowAction("close")}
            aria-label="Tepsiye küçült; koruma çalışmaya devam eder"
            title="Tepsiye küçült; koruma çalışmaya devam eder"
          >
            ×
          </button>
        </div>
      </header>

      {(error ||
        serviceQueryError ||
        !isAdmin ||
        updateAvailable ||
        (dohOn && !dohManaged)) && (
        <div className="notice-stack">
          {!isAdmin && (
            <div className="notice notice-warning">
              <span>
                DPI ve DNS ayarları için uygulamayı yönetici yetkisiyle açmalısın.
              </span>
              <button
                type="button"
                onClick={requestAdminRelaunch}
                disabled={adminRelaunching || updating}
              >
                {adminRelaunching ? "ONAY BEKLENİYOR…" : "YÖNETİCİ OLARAK AÇ"}
              </button>
            </div>
          )}

          {updateAvailable && (
            <div className="notice notice-update">
              <span>Yeni sürüm hazır: v{updateVersion}</span>
              <button
                type="button"
                onClick={installUpdate}
                disabled={updating || busy || serviceBusy}
              >
                {updating ? "GÜNCELLENİYOR…" : "GÜNCELLE VE YENİDEN BAŞLAT"}
              </button>
            </div>
          )}

          {dohOn && !dohManaged && (
            <div className="notice notice-warning" role="status">
              <span>
                Açık DNS ayarı MavroDPI 0.3 günlüğüne ait değil. 0.2.4'ten
                kalmış veya haricen ayarlanmış olabilir; güvenlik için otomatik
                değiştirilmedi.
              </span>
              <button
                type="button"
                onClick={resetExternalDns}
                disabled={
                  resettingExternalDns ||
                  updating ||
                  busy ||
                  serviceBusy ||
                  !isAdmin
                }
              >
                {resettingExternalDns
                  ? "DNS SIFIRLANIYOR…"
                  : "DNS'İ DHCP'YE SIFIRLA"}
              </button>
            </div>
          )}

          {error && (
            <div className="notice notice-error" role="alert">
              <span>{error}</span>
              <button type="button" onClick={() => setError(null)}>
                KAPAT
              </button>
            </div>
          )}

          {serviceQueryError && (
            <div className="notice notice-error" role="alert">
              <span>{serviceQueryError}</span>
            </div>
          )}
        </div>
      )}

      <main className="app-grid">
        <section className="control-column">
          <div className="section-heading">
            <div>
              <span>CONTROL / 01</span>
              <h1>Bağlantı kontrolü</h1>
            </div>
            <div className="selected-profile">
              <span>SEÇİLİ PROFİL</span>
              <strong>
                {profile.name.toLocaleUpperCase("tr-TR")} · {profile.args.join(" ")}
              </strong>
            </div>
          </div>

          <section
            className={`connection-card ${running ? "is-live" : ""} ${busy ? "is-busy" : ""}`}
            aria-live="polite"
          >
            <div className="connection-copy">
              <span className={`eyebrow ${running ? "is-live" : ""}`}>
                <i className="state-dot" />
                {running
                  ? "YEREL DPI SÜRECİ ÇALIŞIYOR"
                  : serviceLocksForeground
                    ? "WINDOWS SERVİSİ KURULU"
                    : "BAŞLATMAYA HAZIR"}
              </span>

              <h2>
                {busy
                  ? "Ayarlar uygulanıyor."
                  : running
                    ? "Yerel atlatma etkin."
                    : serviceLocksForeground
                      ? "Servis modu kurulu."
                      : "Kontrol sende."}
              </h2>

              <p>
                {running
                  ? `${profile.name} (${profile.args.join(" ")}) profili kullanılıyor${dohOn ? " ve Windows şifreli DNS ayarı açık" : "; şifreli DNS şu anda kapalı"}. Trafik uzak bir VPN sunucusuna taşınmıyor.`
                  : serviceLocksForeground
                    ? serviceStatus.needsRepair
                      ? "Windows otomatik başlatma servisi kurulu ancak paket bütünlüğü güncel değil. Ayarlar bölümünden güvenli biçimde onarabilir veya kaldırabilirsin."
                      : `Windows otomatik başlatma servisi ${serviceRunning ? "Dengeli (-5) profiliyle çalışıyor" : "kurulu ancak şu anda çalışmıyor"}. Ayarlar bölümünden başlatabilir veya kaldırabilirsin.`
                    : "Seçili profil erişim engellerinde yerel DPI atlatma uygular. Bu uygulama VPN veya tünel değildir; ağ koşullarına bağlı hız değişimi için sıfır kayıp garantisi vermez."}
              </p>

              <div className="route" aria-label="Yerel doğrudan bağlantı modeli">
                <span className="route-node">
                  <i>01</i>
                  CİHAZ
                </span>
                <span className="route-line">
                  <b />
                </span>
                <span className="route-core">
                  <i>M</i>
                  YEREL DPI
                </span>
                <span className="route-line">
                  <b />
                </span>
                <span className="route-node">
                  <i>03</i>
                  İNTERNET
                </span>
              </div>
            </div>

            <div className="power-zone">
              <button
                type="button"
                className={`power-button ${running ? "is-active" : ""}`}
                onClick={running ? stopProtection : startProtection}
                disabled={
                  busy ||
                  serviceBusy ||
                  updating ||
                  !serviceReady ||
                  !isAdmin ||
                  serviceLocksForeground
                }
                aria-label={
                  running
                    ? "Yerel DPI korumasını durdur"
                    : "Yerel DPI korumasını başlat"
                }
              >
                <span className="power-rings" aria-hidden="true">
                  <i />
                  <i />
                  <i />
                </span>
                <span className="power-icon" aria-hidden="true">
                  <i />
                </span>
                <strong>
                  {busy
                    ? "BEKLE"
                    : running
                      ? "DURDUR"
                      : serviceLocksForeground
                        ? "SERVİS"
                        : "BAŞLAT"}
                </strong>
              </button>
              <small>
                {!isAdmin
                  ? "YÖNETİCİ YETKİSİ GEREKLİ"
                  : serviceLocksForeground
                    ? "AYARLARDAN YÖNET"
                    : running
                      ? "DNS AYARIYLA BİRLİKTE KAPAT"
                      : "SEÇİLİ PROFİLİ UYGULA"}
              </small>
            </div>
          </section>

          <div className="metric-strip">
            <article>
              <span>DPI MOTORU</span>
              <strong className={running ? "metric-live" : ""}>
                <i />
                {running ? "ÇALIŞIYOR" : "KAPALI"}
              </strong>
              <small>
                {running
                  ? `${profile.args.join(" ")}${engineInfo.pid ? ` · PID ${engineInfo.pid}` : ""}`
                  : "Ön plan süreci"}
              </small>
            </article>
            <article>
              <span>ŞİFRELİ DNS</span>
              <strong className={dohOn ? "metric-live" : ""}>
                <i />
                {dohOn ? "AÇIK" : "KAPALI"}
              </strong>
              <small>
                {dohOn
                  ? dohManaged
                    ? "MavroDPI günlüğüyle yönetiliyor"
                    : "Harici / önceki Windows ayarı"
                  : "Windows sistem ayarı"}
              </small>
            </article>
            <article>
              <span>OTURUM</span>
              <strong>{running ? formatDuration(uptime) : "00:00:00"}</strong>
              <small>{running ? "Bu çalıştırma" : "Başlatılmadı"}</small>
            </article>
            <article>
              <span>CİHAZ TRAFİĞİ</span>
              <strong className="network-rate">
                {running ? `↓ ${formatRate(rxRate)}` : "—"}
              </strong>
              <small>{running ? `↑ ${formatRate(txRate)}` : "Tüm arabirimler"}</small>
            </article>
          </div>

          <section className="profiles-section">
            <div className="mini-heading">
              <div>
                <span>PROFILES / 02</span>
                <h2>Çalışma profili</h2>
              </div>
              <p>
                Profili değiştirmek için ön plan sürecini durdur. Servis kendi
                sabit Dengeli (-5) profiliyle çalışır.
              </p>
            </div>

            <div
              className="profiles"
              role="radiogroup"
              aria-label="GoodbyeDPI çalışma profili"
            >
              {PROTECTION_PROFILES.map((item) => {
                const selected = item.id === selectedProfile;
                return (
                  <button
                    type="button"
                    className={`profile-button ${selected ? "is-selected" : ""}`}
                    key={item.id}
                    role="radio"
                    aria-checked={selected}
                    disabled={
                      running || serviceOn || busy || serviceBusy || updating
                    }
                    onClick={() => chooseProfile(item.id)}
                  >
                    <span>{item.badge}</span>
                    <strong>{item.name}</strong>
                    <small>{item.description}</small>
                    <i className="profile-meter">
                      <b style={{ width: item.id === "balanced" ? "58%" : "82%" }} />
                    </i>
                  </button>
                );
              })}
            </div>
          </section>

          <section className="event-strip">
            <div className="event-heading">
              <span>LIVE / GOODBYEDPI OUTPUT</span>
              <div>
                <small>
                  {running
                    ? `OTURUM ↓ ${formatBytes(sessionRx)} · ↑ ${formatBytes(sessionTx)}`
                    : `${logs.length} KAYIT`}
                </small>
                <button type="button" onClick={() => setLogs([])}>
                  KAYITLARI TEMİZLE
                </button>
              </div>
            </div>
            <div className="event-log" aria-live="polite">
              {logs.length === 0 ? (
                <p className="empty-log">Henüz motor çıktısı veya işlem kaydı yok.</p>
              ) : (
                logs.map((entry, index) => (
                  <p key={`${entry}-${index}`}>
                    <i />
                    <span>{entry}</span>
                  </p>
                ))
              )}
              <div ref={logEndRef} />
            </div>
          </section>
        </section>

        <aside className="context-panel">
          <div className="context-tabs" role="tablist" aria-label="Bağlam paneli">
            <button
              type="button"
              className={activeTab === "diagnostics" ? "is-active" : ""}
              role="tab"
              aria-selected={activeTab === "diagnostics"}
              onClick={() => setActiveTab("diagnostics")}
            >
              TANILAMA
            </button>
            <button
              type="button"
              className={activeTab === "settings" ? "is-active" : ""}
              role="tab"
              aria-selected={activeTab === "settings"}
              onClick={() => setActiveTab("settings")}
            >
              AYARLAR
            </button>
          </div>

          {activeTab === "diagnostics" ? (
            <section className="context-content" role="tabpanel">
              <div
                className={`radar ${checking ? "is-running" : ""} ${hasChecked ? "is-done" : ""}`}
                aria-hidden="true"
              >
                <span className="radar-grid" />
                <span className="radar-sweep" />
                <strong>M</strong>
                <i className="point point-a" />
                <i className="point point-b" />
                <i className="point point-c" />
              </div>

              <div className="context-intro">
                <span>REAL CONNECTIVITY CHECK</span>
                <h2>
                  {checking
                    ? "Hedefler kontrol ediliyor."
                    : hasChecked
                      ? `${reachableSiteCount}/${sites.length} hedef erişilebilir.`
                      : "Bağlantıyı ölç."}
                </h2>
                <p>
                  Kontrol, her hedefe TLS doğrulamalı gerçek HTTPS isteği gönderir.
                  Bir HTTP yanıtının alınması bağlantının kurulabildiğini gösterir.
                </p>
              </div>

              <div className="system-checks">
                <article>
                  <span>01</span>
                  <div>
                    <strong>Yönetici yetkisi</strong>
                    <small>Windows işlem yetkisi</small>
                  </div>
                  <b className={isAdmin ? "check-ok" : "check-warning"}>
                    {isAdmin ? "HAZIR" : "GEREKLİ"}
                  </b>
                </article>
                <article>
                  <span>02</span>
                  <div>
                    <strong>DPI motoru</strong>
                    <small>Uygulama içi süreç</small>
                  </div>
                  <b className={running ? "check-ok" : ""}>
                    {running ? "ÇALIŞIYOR" : "KAPALI"}
                  </b>
                </article>
                <article>
                  <span>03</span>
                  <div>
                    <strong>Şifreli DNS</strong>
                    <small>Windows DNS durumu</small>
                  </div>
                  <b className={dohOn ? "check-ok" : ""}>
                    {dohOn ? (dohManaged ? "AÇIK" : "HARİCİ") : "KAPALI"}
                  </b>
                </article>
                <article>
                  <span>04</span>
                  <div>
                    <strong>Windows servisi</strong>
                    <small>Dengeli (-5) otomatik profil</small>
                  </div>
                  <b
                    className={
                      serviceRunning && !serviceStatus.needsRepair
                        ? "check-ok"
                        : serviceOn
                          ? "check-warning"
                          : ""
                    }
                  >
                    {serviceStatus.needsRepair
                      ? "ONARIM GEREKLİ"
                      : serviceRunning
                      ? "ÇALIŞIYOR"
                      : serviceOn
                        ? serviceStateLabel(serviceStatus.state)
                        : "KURULU DEĞİL"}
                  </b>
                </article>
              </div>

              {sites.length > 0 && (
                <div className="site-results" aria-label="Bağlantı testi sonuçları">
                  {sites.map((site) => (
                    <article key={site.host} title={site.detail}>
                      <i className={site.reachable ? "reachable" : "blocked"} />
                      <div>
                        <strong>{site.name}</strong>
                        <small>
                          {site.statusCode ? `HTTP ${site.statusCode}` : site.host}
                          {site.latencyMs !== null ? ` · ${site.latencyMs} ms` : ""}
                        </small>
                      </div>
                      <b className={site.reachable ? "check-ok" : "check-warning"}>
                        {site.reachable ? "ERİŞİLEBİLİR" : "ERİŞİLEMEDİ"}
                      </b>
                    </article>
                  ))}
                </div>
              )}

              <button
                className="primary-button"
                type="button"
                onClick={runDiagnostics}
                disabled={checking}
              >
                {checking ? "GERÇEK KONTROL ÇALIŞIYOR" : "HEDEFLERİ KONTROL ET"}
                <span>{checking ? "···" : "↗"}</span>
              </button>
            </section>
          ) : (
            <section className="context-content" role="tabpanel">
              <div className="context-intro settings-intro">
                <span>SYSTEM & LOCAL SETTINGS</span>
                <h2>Gerçek ayarlar.</h2>
                <p>
                  Servis seçeneği Windows sistemini değiştirir. Hareket tercihi
                  yalnızca bu uygulamada ve bu cihazda saklanır.
                </p>
              </div>

              <div className="settings-list">
                <article className="service-setting">
                  <div>
                    <strong>Windows ile otomatik başlat</strong>
                    <small>
                      {serviceStatus.needsRepair
                        ? "Kurulu paket güncel değil · güvenli onarım gerekli"
                        : serviceRunning
                        ? "Çalışıyor · Dengeli (-5) sabit servis profili"
                        : serviceOn
                          ? `Kurulu fakat çalışmıyor · ${serviceStateLabel(serviceStatus.state)}`
                          : "Arka plan servisi kurulu değil"}
                    </small>
                  </div>
                  <span className="service-actions">
                    {(!serviceOn ||
                      serviceStatus.needsRepair ||
                      !serviceRunning) && (
                      <button
                        type="button"
                        className={`setting-action ${serviceOn ? "is-on" : ""}`}
                        onClick={handleService}
                        disabled={
                          serviceBusy ||
                          busy ||
                          updating ||
                          !serviceReady ||
                          !isAdmin
                        }
                      >
                        {serviceBusy
                          ? "BEKLE"
                          : !serviceOn
                            ? "KUR"
                            : serviceStatus.needsRepair
                              ? "ONAR"
                              : "BAŞLAT"}
                      </button>
                    )}
                    {serviceOn && (
                      <button
                        type="button"
                        className="setting-action is-danger"
                        onClick={removeService}
                        disabled={
                          serviceBusy ||
                          busy ||
                          updating ||
                          !serviceReady ||
                          !isAdmin
                        }
                      >
                        {serviceBusy ? "BEKLE" : "KALDIR"}
                      </button>
                    )}
                  </span>
                </article>

                <article>
                  <div>
                    <strong>Hareketi azalt</strong>
                    <small>Yalnızca arayüz animasyonları</small>
                  </div>
                  <button
                    type="button"
                    className={`switch ${reduceMotion ? "is-on" : ""}`}
                    role="switch"
                    aria-checked={reduceMotion}
                    onClick={() => setReduceMotion((value) => !value)}
                  >
                    <i />
                  </button>
                </article>
              </div>

              <div className="local-boundary">
                <div className="boundary-mark" aria-hidden="true">
                  M
                </div>
                <div>
                  <span>ÇALIŞMA SINIRI</span>
                  <strong>Yerel işlem, doğrudan bağlantı.</strong>
                  <p>
                    GoodbyeDPI bu cihazda çalışır. VPN hesabı, uzak tünel veya
                    trafik aktarım sunucusu kullanılmaz.
                  </p>
                </div>
              </div>
            </section>
          )}
        </aside>
      </main>

      <footer className="safety-bar">
        <div>
          <i />
          <strong>YEREL DPI ATLATMA</strong>
        </div>
        <p>
          VPN veya tünel değildir. Erişim sonucu ve performans ağ koşullarına
          göre değişebilir.
        </p>
        <span>AÇIK KAYNAK · CİHAZDA ÇALIŞIR</span>
      </footer>
    </div>
  );
}
