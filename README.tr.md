# TurkeyDPI

Türkiye'deki operatörler için DPI atlatma aracı. TLS ve HTTP isteklerini TCP segmentlerine
bölerek aradaki inceleme kutusunun alan adını bütün olarak görmesini engeller, isim
çözümlemesini DNS-over-HTTPS üzerinden yaparak operatörün sahte cevap dönmesini önler.

macOS, Windows ve Linux. [English](README.md)

## Hızlı başlangıç

Bağlantınızda hangi profilin çalıştığını bulun:

```bash
turkeydpi doctor
```

Her profili sık engellenen birkaç siteye karşı dener ve hangisinin geçtiğini söyler.
Sonra çalıştırın:

```bash
turkeydpi bypass --preset turk-telekom
```

`127.0.0.1:8844` adresini dinler. Sistem veya tarayıcı HTTP proxy ayarınızı buraya
yönlendirin, ya da TurkeyDPI yapsın:

```bash
turkeydpi set-proxy
```

Bir çökme yüzünden bilgisayarınız çalışmayan bir proxy'ye bakar kalırsa:

```bash
turkeydpi unset-proxy
```

## Kurulum

```bash
cargo install --path cli
```

macOS, Windows ve Linux için hazır dosyalar her
[sürümde](https://github.com/1etu/turkeydpi/releases) mevcut.

## Uygulamalar

**Windows** — `turkeydpi-tray.exe` sadece bir tepsi simgesidir. Sağ tıklayıp proxy'yi
açıp kapatabilir, profil seçebilir veya çıkabilirsiniz. Açtığınızda Windows sistem
proxy ayarı da buna yönlenir, kapattığınızda veya çıktığınızda eski haline döner.

**macOS** — `cd TurkeyDPI-App && ./build.sh` çalıştırıp `TurkeyDPI.app` dosyasını açın.
Menü çubuğu uygulaması; canlı kayıtlar ve açılışta başlatma seçeneği içerir.

## Profiller

| Profil | Ne yapar |
| --- | --- |
| `turk-telekom` | 2. baytta ve alan adının içinde böler |
| `vodafone` | 3. baytta ve alan adının içinde böler, segmentler arası 100µs bekler |
| `superonline` | 1. baytta ve alan adının içinde böler |
| `aggressive` | İki başlık bölmesi, 5 baytlık segmentler, 10ms bekleme |
| `none` | Hiç dokunmaz, karşılaştırma için |

Bütün profiller alan adının *içinde* böler. Farkları, başka nerede kestikleri ve ne kadar
yavaş gönderdikleridir.

## Sadece gerekeni böl

Bölme işlemi gidiş dönüş süresi ekler. Sadece birkaç site için gerekiyorsa listeleyin:

```bash
turkeydpi bypass --domains domains.example.txt
```

Listede olmayan her şey tam hızda, dokunulmadan iletilir. `discord.com` alt alan adlarını
da kapsar; tek bir adresi eşlemek için satırın başına `=` koyun.

## Nasıl çalışır

Operatörler TLS el sıkışmasını inceler. Engelli bir siteye bağlandığınızda ClientHello
paketi alan adını açık metin olarak taşır:

```
İstemci -> Sunucu: TLS ClientHello
  Kayıt başlığı:  16 03 03 [uzunluk]
  El sıkışma tipi: 01 (ClientHello)
  Uzantılar:
    SNI (0x0000): "discord.com"    <- DPI kutusu bunu okur
```

Bunu engel listesiyle karşılaştırır ve bağlantıyı keser.

TCP bir akıştır. Sunucu verinin tek segmentte mi yirmi segmentte mi geldiğini umursamaz,
çünkü ayrıştırmadan önce birleştirir. Birçok DPI kutusu ise umursar: segmentleri tek tek
inceler ve alan adı ikiye bölündüğünde eşleştiremez.

```
Normal:    [16 03 03 .. 01 .. "discord.com" ..]     tek segment, engellenir

Bölünmüş:  [16 03] [03 .. 01 .. "disc"] ["ord.com" ..]
                                    |
                                    alan adı segmentlere bölündü
```

Sunucu bunları birleştirir ve el sıkışma normal şekilde tamamlanır.

Aynı mantık düz HTTP için de geçerlidir; orada alan adı `Host:` başlığındadır ve istek o
değerin içinden bölünür.

### DNS

Operatörler DNS cevaplarını da bozar. TurkeyDPI ismi DNS-over-HTTPS ile çözer (sırasıyla
Cloudflare, Quad9, Google), sunucunun verdiği TTL süresine uyar ve pes etmeden önce dönen
bütün adresleri dener. Sistem çözücüsüne **geri dönmez**, çünkü bozulan şey tam olarak
odur.

## Bu araç ne yapmaz

- VPN **değildir**, anonimlik aracı **değildir**. Operatörünüz bağlandığınız her adresi
  görmeye devam eder.
- Zaten şifreli olmayan hiçbir şeyi şifrelemez.
- IP adresi üzerinden yapılan engellemeye karşı işe yaramaz, sadece alan adı incelemesine
  karşı çalışır.
- Sizi aktif olarak yoklayan bir operatöre karşı koruma sağlamaz.

## Derleme

```bash
cargo build --release
cargo test --workspace
```

## Lisans

MIT
