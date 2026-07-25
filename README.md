# MavroDPI

MavroDPI is a Windows x64 desktop interface for running a locally bundled
GoodbyeDPI engine and, where Windows supports it, configuring native encrypted
DNS. It is built with Tauri, Rust, React, and TypeScript.

## Türkçe

MavroDPI bir VPN, proxy veya tünel değildir. Trafiği uzak bir sunucuya
yönlendirmez; GoodbyeDPI ve WinDivert cihazın üzerinde çalışır. Yönetici
yetkisi, WinDivert sürücüsü ve Windows ağ/DNS ayarları için gereklidir.

### Destek

- **Windows 11 x64:** DPI motoru ve yerel Windows DNS-over-HTTPS (DoH) akışı
  desteklenir. Uygulama mevcut DNS ayarını kaydeder ve durdururken geri
  yüklemeyi dener.
- **Windows 10 x64:** DPI motoru kullanılabilir. Windows sürümü gerekli yerel
  DoH komutlarını sağlamıyorsa şifreli DNS etkinleştirilemez; yerel DoH
  garantisi verilmez.
- **0.2.4'ten güncelleme:** 0.3.0 yalnızca kendisinin kaydettiği DNS
  değişikliklerini geri yükleyebilir. 0.2.4 özgün DNS ayarını günlüklemediği
  için güncelleme öncesindeki değeri güvenli biçimde yeniden oluşturmak mümkün
  değildir; mevcut DNS ayarı dış yapılandırma olarak korunur. Arayüz bu durumu
  **Harici** olarak gösterir ve DHCP'ye sıfırlamayı yalnız kullanıcı açıkça
  isterse yapar.

### Profiller

- **Dengeli (`-5`):** Çoğu bağlantı için başlangıç profili.
- **Uyumluluk (`-6`):** `-5` sonuç vermediğinde denenebilen, GoodbyeDPI
  0.2.2'nin `wrong-seq`, `reverse-frag` ve `max-payload` seçeneklerini kullanan
  alternatif profil.

### Sınırlar

- GoodbyeDPI, DPI tabanlı müdahalelere karşı çalışır; doğrudan IP adresi
  engellerini aşmaz.
- QUIC/HTTP3 davranışı tarayıcıya ve ağa bağlıdır; GoodbyeDPI esas olarak
  desteklediği TCP tabanlı trafiğe müdahale eder.
- Sonuçlar ISS'ye, hedefe ve ağ yapılandırmasına göre değişir.
- Paket yakalama ve yeniden işleme maliyetsiz değildir. MavroDPI sıfır ek yük,
  sıfır gecikme veya her ağda erişim garantisi vermez.
- Windows paketleri henüz Authenticode ile imzalanmadığından SmartScreen
  "Bilinmeyen yayıncı" uyarısı gösterebilir. Dosya bütünlüğünü sürümdeki
  `SHA256SUMS.txt` ile doğrulayabilirsiniz.

### Geliştirme

Windows üzerinde Node.js 20, Rust stable ve Tauri'nin Windows önkoşulları
gereklidir.

```powershell
npm ci
npm run build
cargo test --manifest-path src-tauri/Cargo.toml --locked
.\scripts\prepare-service.ps1
npm run tauri build
```

## English

MavroDPI is not a VPN, proxy, or tunnel. It does not route traffic through a
remote server; the bundled GoodbyeDPI and WinDivert components run locally.
Administrator access is required for the WinDivert driver and Windows
network/DNS configuration.

- **Windows 11 x64:** supports the DPI engine and the native Windows
  DNS-over-HTTPS flow, including best-effort restoration of the previous DNS
  configuration.
- **Windows 10 x64:** supports the DPI engine, but native DoH is not guaranteed
  when the required Windows commands are unavailable.
- **Updating from 0.2.4:** 0.3.0 can restore only DNS changes it journals
  itself. Because 0.2.4 did not record the original DNS configuration, that
  earlier value cannot be reconstructed safely; the current DNS setting is
  preserved as external configuration. The UI labels it **External** and
  resets it to DHCP only after an explicit user action.
- **Profiles:** Balanced maps to GoodbyeDPI `-5`; Compatibility maps to `-6`.
- **Limitations:** IP-address blocks are outside GoodbyeDPI's DPI-focused
  scope. QUIC/HTTP3 behavior depends on the client and network; GoodbyeDPI
  primarily acts on supported TCP traffic.
  Results and performance depend on the network. No zero-overhead or universal
  access claim is made.
- The Windows packages are not Authenticode-signed yet, so SmartScreen may
  report an unknown publisher. Verify downloads against `SHA256SUMS.txt`.

## License

MavroDPI application code is available under the [MIT License](LICENSE).
Bundled native components retain their own licenses; see
[Third-party notices](THIRD_PARTY_NOTICES.md).
