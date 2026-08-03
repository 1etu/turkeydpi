# TurkeyDPI

DPI bypass for Turkish ISPs. Splits TLS and HTTP requests across TCP segments so the
middlebox never sees a complete hostname, and resolves names over DNS-over-HTTPS so the
ISP resolver cannot lie to you.

macOS, Windows and Linux. [Türkçe](README.tr.md)

## Quick start

Find the preset that works on your connection:

```bash
turkeydpi doctor
```

It tries every preset against a handful of commonly blocked sites and tells you which one
got through. Then run it:

```bash
turkeydpi bypass --preset turk-telekom
```

Listens on `127.0.0.1:8844`. Point your system or browser HTTP proxy there, or let
TurkeyDPI do it:

```bash
turkeydpi set-proxy
```

If a crash ever leaves your machine pointing at a proxy that is not running:

```bash
turkeydpi unset-proxy
```

## Install

```bash
cargo install --path cli
```

Prebuilt binaries for macOS, Windows and Linux are attached to each
[release](https://github.com/1etu/turkeydpi/releases).

## Apps

**Windows** — `turkeydpi-tray.exe` is a tray icon and nothing else. Right click it to turn
the proxy on or off, pick a preset, or quit. Turning it on also points the Windows system
proxy at it; turning it off or quitting puts the setting back.

**macOS** — `cd TurkeyDPI-App && ./build.sh`, then open `TurkeyDPI.app`. Menu bar app with
per-container control, live logs and a launch-at-login toggle.

## Presets

| Preset | What it does |
| --- | --- |
| `turk-telekom` | Split at byte 2 and inside the hostname |
| `vodafone` | Split at byte 3 and inside the hostname, 100µs between segments |
| `superonline` | Split at byte 1 and inside the hostname |
| `aggressive` | Two header splits, 5-byte segments, 10ms between segments |
| `none` | Forward untouched, useful as a control |

Every preset splits *inside* the hostname. They differ in where else they cut and how
slowly they send.

## Only fragment what you have to

Fragmentation costs round trips. If you only need it for a handful of sites, list them:

```bash
turkeydpi bypass --domains domains.example.txt
```

Anything not on the list is forwarded untouched at full speed. `discord.com` covers its
subdomains too; prefix a line with `=` to match one exact host.

## How it works

Turkish ISPs inspect the TLS handshake. When you connect to a blocked site, the
ClientHello carries the hostname in plaintext:

```
Client -> Server: TLS ClientHello
  Record header:  16 03 03 [length]
  Handshake type: 01 (ClientHello)
  Extensions:
    SNI (0x0000): "discord.com"    <- the DPI box reads this
```

It matches that against a blocklist and kills the connection.

TCP is a stream. The server does not care whether your data arrives in one segment or
twenty, because it reassembles before parsing. Many DPI boxes do care: they inspect
segments individually and give up when the hostname is cut in half.

```
Normal:      [16 03 03 .. 01 .. "discord.com" ..]     one segment, blocked

Fragmented:  [16 03] [03 .. 01 .. "disc"] ["ord.com" ..]
                 |                    |
                 |                    hostname split across segments
                 record header cut before the handshake type
```

The server reassembles and completes the handshake normally.

The same idea applies to plaintext HTTP, where the hostname sits in the `Host:` header,
and the request is split inside that value.

### DNS

ISPs also poison DNS. TurkeyDPI resolves over DNS-over-HTTPS (Cloudflare, Quad9, Google,
in that order), honours the TTL the resolver returns, and tries every address it gets back
before giving up. It does **not** fall back to the system resolver, because that is the
thing being poisoned.

## Configuration

Most people never need a config file. If you want one:

```bash
turkeydpi gen-config > turkeydpi.toml
turkeydpi validate turkeydpi.toml
```

Unknown keys are rejected rather than silently ignored. See
[config.example.toml](config.example.toml).

## What this does not do

- It is **not** a VPN and **not** anonymity software. Your ISP still sees every address
  you connect to.
- It does not encrypt anything that was not already encrypted.
- It does not help against blocking done by IP address, only by hostname inspection.
- It offers no protection against an ISP that actively probes or fingerprints you.

See [SECURITY.md](SECURITY.md).

## Layout

```
cli/           turkeydpi binary
engine/        hostname parsing, split strategies, DoH, reachability probe
backend/       HTTP CONNECT proxy and SOCKS5 proxy
control/       daemon IPC
sysproxy/      system proxy settings per platform
windows-app/   Windows tray app
TurkeyDPI-App/ macOS menu bar app
```

## Build

```bash
cargo build --release
cargo test --workspace
```

## License

MIT
