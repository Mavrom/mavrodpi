import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { PROTECTION_ARGS } from "./profiles";
import "./App.css";

interface NetSample  { rx: number; tx: number; }
interface SiteStatus { name: string; host: string; reachable: boolean; }

function formatBytes(n: number): string {
  if (n < 1024)            return `${n} B`;
  if (n < 1024 ** 2)       return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 ** 3)       return `${(n / 1024 ** 2).toFixed(1)} MB`;
  return `${(n / 1024 ** 3).toFixed(2)} GB`;
}
function formatRate(b: number) { return `${formatBytes(b)}/sn`; }
function formatDuration(sec: number): string {
  const h = Math.floor(sec / 3600);
  const m = Math.floor((sec % 3600) / 60);
  const s = Math.floor(sec % 60);
  const p = (x: number) => String(x).padStart(2, "0");
  return h > 0 ? `${h}:${p(m)}:${p(s)}` : `${p(m)}:${p(s)}`;
}

export default function App() {
  /* ── koruma durumu ── */
  const [running, setRunning]   = useState(false);
  const [busy, setBusy]         = useState(false);
  const [isAdmin, setIsAdmin]   = useState(true);
  const [dohOn, setDohOn]       = useState(false);
  const [error, setError]       = useState<string | null>(null);

  /* ── servis ── */
  const [serviceOn, setServiceOn]     = useState(false);
  const [serviceBusy, setServiceBusy] = useState(false);

  /* ── güncelleme ── */
  const [updateAvailable, setUpdateAvailable] = useState(false);
  const [updateVersion, setUpdateVersion]     = useState("");
  const [updating, setUpdating]               = useState(false);

  /* ── tema ── */
  const [theme, setTheme] = useState<"dark" | "light">(
    () => (localStorage.getItem("theme") as "dark" | "light") || "dark"
  );

  /* ── istatistikler ── */
  const [uptime, setUptime]       = useState(0);
  const [rxRate, setRxRate]       = useState(0);
  const [txRate, setTxRate]       = useState(0);
  const [sessionRx, setSessionRx] = useState(0);
  const [sessionTx, setSessionTx] = useState(0);

  /* ── bağlantı testi ── */
  const [sites, setSites]       = useState<SiteStatus[]>([]);
  const [checking, setChecking] = useState(false);

  /* ── log ── */
  const [logs, setLogs]       = useState<string[]>([]);
  const [logOpen, setLogOpen] = useState(false);

  const startedAtRef = useRef<number>(0);
  const prevNetRef   = useRef<{ rx: number; tx: number; t: number } | null>(null);
  const logEndRef    = useRef<HTMLDivElement>(null);

  /* tema uygula */
  useEffect(() => {
    document.documentElement.setAttribute("data-theme", theme);
    localStorage.setItem("theme", theme);
  }, [theme]);

  const addLog = useCallback((msg: string) => {
    setLogs(prev => [...prev.slice(-300), msg]);
  }, []);

  /* ── başlat ── */
  const start = useCallback(async () => {
    setError(null);
    setBusy(true);
    try {
      await invoke("enable_doh");
      setDohOn(true);
      addLog("Şifreli DNS (DoH) açıldı — DNS engellemeleri aşılıyor.");
      await invoke("start_protection", { args: PROTECTION_ARGS });
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }, [addLog]);

  /* ── durdur ── */
  async function stop() {
    setError(null);
    setBusy(true);
    try {
      await invoke("stop_protection").catch(() => {});
      await invoke("disable_doh").catch(() => {});
      setDohOn(false);
      addLog("Koruma ve şifreli DNS kapatıldı.");
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  /* ── ilk yükleme ── */
  useEffect(() => {
    invoke<boolean>("get_status").then(setRunning).catch(() => {});
    invoke<boolean>("is_admin").then(setIsAdmin).catch(() => {});
    invoke<boolean>("doh_status").then(setDohOn).catch(() => {});
    invoke<boolean>("service_installed").then(setServiceOn).catch(() => {});

    // Arka planda güncelleme kontrolü
    check().then(update => {
      if (update?.available) {
        setUpdateAvailable(true);
        setUpdateVersion(update.version);
      }
    }).catch(() => {});

    const u1 = listen<boolean>("gdpi-status", e => setRunning(e.payload));
    const u2 = listen<string>("gdpi-log", e =>
      setLogs(prev => [...prev.slice(-300), e.payload])
    );
    return () => { u1.then(f => f()); u2.then(f => f()); };
  }, []);

  async function installUpdate() {
    setUpdating(true);
    try {
      const update = await check();
      if (update?.available) {
        await update.downloadAndInstall();
        await relaunch();
      }
    } catch (e) {
      setError(String(e));
      setUpdating(false);
    }
  }

  /* ── uptime + ağ ── */
  useEffect(() => {
    if (!running) {
      setUptime(0); setRxRate(0); setTxRate(0);
      prevNetRef.current = null;
      return;
    }
    startedAtRef.current = Date.now();
    setSessionRx(0); setSessionTx(0);
    const id = setInterval(async () => {
      setUptime((Date.now() - startedAtRef.current) / 1000);
      try {
        const s   = await invoke<NetSample>("net_stats");
        const now = Date.now();
        const prev = prevNetRef.current;
        if (prev) {
          const dt  = (now - prev.t) / 1000;
          const dRx = Math.max(0, s.rx - prev.rx);
          const dTx = Math.max(0, s.tx - prev.tx);
          if (dt > 0) {
            setRxRate(dRx / dt);
            setTxRate(dTx / dt);
            setSessionRx(v => v + dRx);
            setSessionTx(v => v + dTx);
          }
        }
        prevNetRef.current = { rx: s.rx, tx: s.tx, t: now };
      } catch {}
    }, 1000);
    return () => clearInterval(id);
  }, [running]);

  useEffect(() => {
    logEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [logs]);

  /* ── bağlantı testi ── */
  async function checkSites() {
    setChecking(true);
    setSites([]);
    try {
      const result = await invoke<SiteStatus[]>("check_sites");
      setSites(result);
    } catch (e) {
      addLog(`Bağlantı testi hatası: ${e}`);
    } finally {
      setChecking(false);
    }
  }

  /* ── servis kur/kaldır ── */
  async function handleService() {
    setServiceBusy(true);
    setError(null);
    try {
      if (serviceOn) {
        await invoke("uninstall_service");
        setServiceOn(false);
        addLog("Otomatik başlatma servisi kaldırıldı.");
      } else {
        await invoke("install_service");
        setServiceOn(true);
        addLog("Servis kuruldu ve başlatıldı — PC açıldığında otomatik koruma başlayacak.");
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setServiceBusy(false);
    }
  }

  const active = running || dohOn;

  return (
    <div className="app">

      {/* ── Header ── */}
      <header className="header">
        <div className="brand">
          <div className="brand-mark">⛩</div>
          <div>
            <h1 className="brand-name">MavroDPI</h1>
            <span className="tagline">Engelsiz internet</span>
          </div>
        </div>
        <div className="header-right">
          <div className={`pill ${active ? "on" : "off"}`}>
            <span className="pill-dot" />
            {active ? "Aktif" : "Kapalı"}
          </div>
          <button
            className="icon-btn"
            title="Tema değiştir"
            onClick={() => setTheme(t => t === "dark" ? "light" : "dark")}
          >
            {theme === "dark" ? "☀" : "☾"}
          </button>
        </div>
      </header>

      {/* ── Yönetici uyarısı ── */}
      {!isAdmin && (
        <div className="banner warn">
          <span>⚠ Yönetici yetkisi gerekiyor</span>
          <button onClick={() => invoke("relaunch_admin")}>Yönetici olarak başlat</button>
        </div>
      )}

      {/* ── Güncelleme ── */}
      {updateAvailable && (
        <div className="banner info">
          <span>Yeni sürüm hazır: v{updateVersion}</span>
          <button onClick={installUpdate} disabled={updating}>
            {updating ? "Güncelleniyor…" : "Güncelle ve Yeniden Başlat"}
          </button>
        </div>
      )}

      {/* ── Orb ── */}
      <section className="hero">
        <button
          className={`orb ${active ? "active" : ""}`}
          onClick={active ? stop : start}
          disabled={busy || !isAdmin}
        >
          <span className="orb-ring" />
          <span className="orb-icon">{active ? "✓" : "⏻"}</span>
          <span className="orb-label">
            {busy ? "Lütfen bekle…" : active ? "Korumadasın" : "Tüm Engelleri Kaldır"}
          </span>
        </button>
        <p className="hero-hint">
          {active
            ? "DNS şifreleme + DPI atlatma katmanları aktif."
            : "Tek tıkla tüm erişim engellerini kaldır."}
        </p>
      </section>

      {/* ── İstatistikler ── */}
      <div className="stats-grid">
        <div className="stat-card">
          <span className="stat-label">Şifreli DNS</span>
          <span className={`stat-val ${dohOn ? "good" : "dim"}`}>{dohOn ? "Açık" : "Kapalı"}</span>
        </div>
        <div className="stat-card">
          <span className="stat-label">DPI Koruması</span>
          <span className={`stat-val ${running ? "good" : "dim"}`}>{running ? "Açık" : "Kapalı"}</span>
        </div>
        <div className="stat-card">
          <span className="stat-label">Çalışma Süresi</span>
          <span className="stat-val">{running ? formatDuration(uptime) : "—"}</span>
        </div>
        <div className="stat-card">
          <span className="stat-label">Anlık Hız</span>
          <span className="stat-val">
            {running ? <>↓ {formatRate(rxRate)}<br />↑ {formatRate(txRate)}</> : "—"}
          </span>
        </div>
        <div className="stat-card wide">
          <span className="stat-label">Oturum Trafiği</span>
          <span className="stat-val">
            {running
              ? `↓ ${formatBytes(sessionRx)}  ·  ↑ ${formatBytes(sessionTx)}`
              : "—"}
          </span>
        </div>
      </div>

      {/* ── Bağlantı Testi ── */}
      <section className="card">
        <div className="card-head">
          <div>
            <div className="card-title">Bağlantı Testi</div>
            <div className="card-sub">Engellenen siteleri kontrol et</div>
          </div>
          <button className="btn btn-outline" onClick={checkSites} disabled={checking}>
            {checking ? "Test ediliyor…" : "Test Et"}
          </button>
        </div>

        {sites.length === 0 && !checking && (
          <p className="hint">
            "Test Et" butonuna basarak hangi sitelerin engellendiğini görebilirsin.
          </p>
        )}
        {checking && (
          <div className="checking-row">
            <span className="spinner" />
            Siteler kontrol ediliyor…
          </div>
        )}
        {sites.length > 0 && (
          <div className="sites-grid">
            {sites.map(s => (
              <div key={s.host} className="site-item">
                <span className={`dot ${s.reachable ? "ok" : "blocked"}`} />
                <span className="site-name">{s.name}</span>
                <span className={`site-tag ${s.reachable ? "ok" : "blocked"}`}>
                  {s.reachable ? "Açık" : "Engelli"}
                </span>
              </div>
            ))}
          </div>
        )}
      </section>

      {/* ── Servis ── */}
      <section className="card">
        <div className="card-head">
          <div>
            <div className="card-title">Otomatik Başlatma Servisi</div>
            <div className="card-sub">
              {serviceOn
                ? "PC her açıldığında koruma otomatik devreye girer, uygulama açık olmak zorunda değil."
                : "Kurunca PC her açıldığında arka planda otomatik çalışır."}
            </div>
          </div>
          <div className="svc-actions">
            <span className={`badge ${serviceOn ? "green" : "neutral"}`}>
              {serviceOn ? "● Kurulu" : "○ Kurulu değil"}
            </span>
            <button
              className={`btn ${serviceOn ? "btn-danger" : "btn-primary"}`}
              onClick={handleService}
              disabled={serviceBusy || !isAdmin}
            >
              {serviceBusy ? "Bekle…" : serviceOn ? "Kaldır" : "Kur"}
            </button>
          </div>
        </div>
      </section>

      {/* ── Hata ── */}
      {error && <div className="banner error">{error}</div>}

      {/* ── Log ── */}
      <section className="card no-pad">
        <button className="collapse-btn" onClick={() => setLogOpen(v => !v)}>
          <span>Kayıt / Günlük</span>
          <span className={`chev ${logOpen ? "open" : ""}`}>▾</span>
        </button>
        {logOpen && (
          <div className="log-body">
            <div className="log-toolbar">
              <span className="log-count">{logs.length} satır</span>
              <button className="btn-ghost" onClick={() => setLogs([])}>Temizle</button>
            </div>
            <div className="log-scroll">
              {logs.length === 0
                ? <span className="log-empty">Henüz kayıt yok.</span>
                : logs.map((l, i) => <div key={i} className="log-line">{l}</div>)
              }
              <div ref={logEndRef} />
            </div>
          </div>
        )}
      </section>

    </div>
  );
}
