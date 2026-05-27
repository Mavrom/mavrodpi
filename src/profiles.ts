// GoodbyeDPI DPI/SNI atlatma argümanları.
// -5: GoodbyeDPI'nin önerilen varsayılan modu (-f 2 -e 2 --auto-ttl
// --reverse-frag --max-payload). Hızlı ve uyumlu. DNS engellemesi DoH ile
// çözüldüğü için burada DNS yönlendirmesine gerek yok.
export const PROTECTION_ARGS: string[] = ["-5"];
