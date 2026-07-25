# MavroDPI Setup wrapper

Bu klasör, gerçek Tauri NSIS kurulumunu başlatan bağımsız Electron
wrapper'ıdır. Wrapper kendisi yönetici yetkisiyle başlamaz; kullanıcı
**Kurulumu başlat** dediğinde paketin içindeki doğrulanmış NSIS dosyasını sabit
`/S` argümanıyla Windows UAC üzerinden çalıştırır. Mevcut bir MavroDPI
kurulumu algılanırsa servis ve kullanıcı ayarlarını koruyan Tauri `/UPDATE`
modu da eklenir.

Arayüz yüzde veya süre tahmini üretmez. Yalnız doğrulanabilen durumları
gösterir: payload doğrulama, UAC bekleme, NSIS sürecinin başlaması ve çıkış
kodu, kurulu `MavroDPI.exe` ile Windows kaldırma kaydının doğrulanması.

## Komutlar

```powershell
npm ci
npm run verify
npm run build:ui
```

`build:ui`, payload olmadan açılabilir paket dizini üretir ve kur düğmesini
güvenli biçimde devre dışı bırakır.

Gerçek release için önce ana Tauri uygulamasını derle:

```powershell
cd ..
npm run tauri build
cd installer
npm run build
```

Release derlemesi yalnız
`../src-tauri/target/release/bundle/nsis/MavroDPI_0.3.2_x64-setup.exe`
eşleşmesini kabul eder; sürüm, dosya boyutu ve SHA256 değerlerini içeren
`payload-manifest.json` dosyasını üretir. Wrapper çalışırken manifest ile
payload'ı yeniden doğrular, kurulumdan sonra registry `DisplayVersion`
değerinin `0.3.2` olduğunu denetler ve
`release/MavroDPI-Setup.exe` üretir.
