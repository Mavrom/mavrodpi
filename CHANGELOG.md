# Changelog

## 0.3.0 — 2026-07-24

- Demo arayüzü gerçek GoodbyeDPI 0.2.2 ve WinDivert motoruna bağlandı.
- GoodbyeDPI 0.2.2 ile uyumlu Dengeli (`-5`) ve Uyumluluk (`-6`) profilleri
  eklendi.
- Gerçek TLS/HTTPS bağlantı tanılaması, motor PID/durum takibi ve cihaz ağ
  sayaçları eklendi.
- Windows 11 için geri alınabilir yerel DNS-over-HTTPS akışı eklendi.
- Özgün DNS ve DoH sunucu ayarlarını günlükleyip kapanma, çökme ve kaldırmada
  geri yükleyen güvenli ağ işlemi eklendi.
- Servis yönetimi yalnızca MavroDPI'ye ait süreç ve dosyalarla sınırlandı;
  servis güncellemelerine doğrulamalı staging ve geri alma eklendi.
- Ön plan ve servis motorları, üst süreç kapanırsa Windows tarafından
  sonlandırılan ayrı süreç gruplarına alındı.
- Native dosya SHA-256 doğrulaması, servis helper PE doğrulaması ve sıkı CSP
  eklendi.
- OLED siyah/beyaz/turuncu çerçevesiz arayüz ve özel kurulum deneyimi eklendi.
- 0.2.4 güncellemesinde yalnız kurulum dizinindeki eski foreground motorunu
  kapatan exact-path geçiş temizliği eklendi. 0.2.4 özgün DNS'i
  günlüklemediğinden bu sürüm güncelleme öncesi DNS değerini yeniden
  oluşturmaz; yalnız 0.3.0'ın kendi günlüğünü geri yükler. Günlüksüz açık DNS
  arayüzde Harici olarak işaretlenir ve ancak kullanıcı isterse DHCP'ye
  sıfırlanır.
- Windows paketlerinin henüz Authenticode ile imzalanmadığı ve SmartScreen'in
  "Bilinmeyen yayıncı" uyarısı gösterebileceği açıklandı.

Teknik sınırlar ve üçüncü taraf lisansları için `README.md` ile
`THIRD_PARTY_NOTICES.md` dosyalarına bakın.
