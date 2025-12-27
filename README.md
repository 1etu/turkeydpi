# TurkeyDPI

DPI bypass for Turkish ISPs. Fragments TLS/HTTP packets to evade SNI-based blocking.

## Install

```bash
cargo install --path cli
```

## Usage

```bash
turkeydpi bypass -v
```

Listens on `127.0.0.1:8844`. Set system HTTP/HTTPS proxy to this address.

### Presets

```bash
turkeydpi bypass --preset aggressive    # recommended
turkeydpi bypass --preset turk-telekom
turkeydpi bypass --preset vodafone
turkeydpi bypass --preset superonline
```

## macOS App

```bash
cd TurkeyDPI-App && ./build.sh
open TurkeyDPI.app
```

Native menu bar app with one-click proxy toggle.

## How it works

### The Problem

Turkish ISPs inspect your traffic using Deep Packet Inspection (DPI). When you connect to the blocked websites (for ex: `discord.com`), the TLS handshake looks like this:

```
Client → Server: TLS ClientHello
  ├─ TLS Record Header: 16 03 03 [length]
  ├─ Handshake Type: 01 (ClientHello)
  ├─ Version, Random, Session ID...
  └─ Extensions:
       └─ SNI (type 0x0000): "discord.com"  ← DPI reads this
```

The DPI box sees `discord.com` in plaintext, matches it against a blocklist, and kills the connection.

### The Solution

TCP is a stream protocol. The server doesn't care if data arrives in one packet or twenty—it reassembles everything. But DPI boxes are stateless and inspect packets individually.

We exploit this:

```
Normal:     [TLS Header + ClientHello + SNI "discord.com"] → DPI blocks

Fragmented: [TLS Hea] [der + Cli] [entHello] [+ SNI "dis] [cord.com"]
                ↓
            DPI sees 5 incomplete packets, can't extract SNI
                ↓
            Server reassembles → valid TLS handshake
```

### TLS Record Structure

```
Byte:   0      1-2     3-4      5+
      ┌────┬────────┬────────┬─────────────────────┐
      │ 16 │ 03 03  │ length │ Handshake data...   │
      └────┴────────┴────────┴─────────────────────┘
        │      │        │
        │      │        └─ 2 bytes: record length
        │      └─ TLS version (0x0303 = TLS 1.2)
        └─ Content type (0x16 = Handshake)
```

The SNI extension sits inside the handshake data, typically 40-200 bytes in. We parse the ClientHello to find the exact byte offset of the hostname.

### Fragment Strategy

Split point matters. Turkish DPI specifically looks for:
1. Content type `0x16` (handshake)
2. Handshake type `0x01` (ClientHello)
3. SNI extension with readable hostname

We split *before* the handshake type is visible:

```
Original:  [16] [03 03] [00 xx] [01 00 00 ... SNI ...]
                                 ↑
                                 Handshake type

Split:     [16 03] [03 00 xx 01 00 ... SNI ...]
               ↑
               DPI never sees complete record header
```

Or split the SNI hostname itself:

```
SNI field: [...] [00 0b] "discord.com" [...]
                          ↓
Split:     [...] [00 0b] "disc" | "ord.com" [...]
```

### HTTP Host Header

Same principle for HTTP:

```
GET / HTTP/1.1
Host: twitter.com    ← DPI reads this
Connection: close
```

Fragment within the Host value:

```
GET / HTTP/1.1
Host: twit  →  [first packet]
ter.com     →  [second packet]
Connection: close
```

### DNS Bypass

ISPs also poison DNS. We use DNS-over-HTTPS (DoH) to Cloudflare:

```
Normal:    DNS query for discord.com → ISP returns fake IP
With DoH:  HTTPS POST to 1.1.1.1/dns-query → encrypted → real IP
```

### Timing

Some DPI boxes buffer packets briefly hoping to reassemble. Adding 10-50ms delay between fragments defeats this:

```
[fragment 1] ──────────────────────────────→
                    wait 10ms
             [fragment 2] ────────────────→
                    wait 10ms
                          [fragment 3] ──→
```

The DPI buffer expires before reassembly completes.

### Techniques

- **SNI fragmentation**: Split TLS ClientHello across TCP segments
- **Host header fragmentation**: Split HTTP Host header
- **Segment size control**: Force small MSS via socket options
- **Timing jitter**: Delays between fragments
- **DoH**: Encrypted DNS resolution

## Build

```bash
cargo build --release
```

## Structure

```
cli/        CLI binary
engine/     Bypass logic, transforms
backend/    Proxy server, TUN (wip)
control/    Daemon IPC
```

## License

MIT
