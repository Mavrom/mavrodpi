# MavroDPI — Claude Talimatları

## Proje Nedir
Tauri v2 + React + Rust ile yapılmış Windows masaüstü uygulaması. GoodbyeDPI üzerinden DPI atlatma ve şifreli DNS (DoH) sağlar.

## Kritik Dosyalar
- `src-tauri/tauri.conf.json` — uygulama config, versiyon, updater endpoint
- `src-tauri/Cargo.toml` — Rust bağımlılıkları ve versiyon
- `src-tauri/src/service.rs` — Windows Scheduled Task kurulumu
- `src-tauri/src/engine.rs` — GoodbyeDPI süreç yönetimi
- `src-tauri/src/lib.rs` — plugin kayıtları ve uygulama giriş noktası
- `src/App.tsx` — React UI
- `.github/workflows/release.yml` — GitHub Actions release pipeline

## GitHub
- Repo: https://github.com/Mavrom/mavrodpi
- GitHub Actions her `v*` tag push'unda otomatik build alır ve release yayınlar
- Signing private key GitHub secret olarak kayıtlı: `TAURI_SIGNING_PRIVATE_KEY`

## Updater
- Public key `tauri.conf.json` içinde `plugins.updater.pubkey` alanında
- Endpoint: `https://github.com/Mavrom/mavrodpi/releases/latest/download/latest.json`
- Uygulama açılışta arka planda kontrol eder, yeni sürüm varsa mavi banner gösterir

## Yeni Sürüm Çıkarma (Claude Yapacak)
Kullanıcı yeni sürüm istemesi halinde şu adımları eksiksiz yap, hiçbir şeyi kullanıcıya bırakma:

1. **Versiyon numarasını iki dosyada birden güncelle:**
   - `src-tauri/tauri.conf.json` → `"version"` alanı
   - `src-tauri/Cargo.toml` → `version` alanı
   Versiyon formatı: `MAJOR.MINOR.PATCH` (örn. `0.2.0`)

2. **Değişiklikleri commit et:**
   ```
   git add src-tauri/tauri.conf.json src-tauri/Cargo.toml
   git commit -m "chore: bump version to vX.Y.Z"
   ```

3. **Tag oluştur ve push et:**
   ```
   git tag vX.Y.Z
   git push origin master
   git push origin vX.Y.Z
   ```

4. **GitHub Actions durumunu kontrol et:**
   ```
   gh run list --repo Mavrom/mavrodpi --limit 1
   ```
   Build tamamlanınca release otomatik yayınlanır.

## Dev Ortamı
```powershell
npm install
npm run tauri dev    # geliştirme modu (yönetici terminalde)
npm run tauri build  # lokal build (sadece test için, release için GitHub Actions kullan)
```

## Mimari Notlar
- Servis: `C:\ProgramData\MavroDPI` klasörüne kopyalanır, Windows Scheduled Task olarak `SYSTEM` ile çalışır
- `RunOnlyIfNetworkAvailable` KASTEN kaldırıldı — açılışta ağ hazır olmadan önce çalışması gerekiyor
- Uygulama kapatıldığında tepsiye küçülür, tamamen çıkmak için tepsi menüsünden "Çıkış"
- DoH (DNS-over-HTTPS) servis tarafından değil, sadece uygulama açıkken etkin
