# Changelog

## 0.2.0

### Added

- `turkeydpi doctor` tries every preset against blocked sites and tells you which one
  works, with `--json` for reports.
- Windows tray app (`turkeydpi-tray.exe`): turn the proxy on or off, pick a preset, quit.
- Windows and Linux builds in CI and in releases.
- `--domains` limits fragmentation to a list of hosts so the rest of your traffic is
  untouched.
- `set-proxy` and `unset-proxy` for the system proxy, including a way out when a crash
  leaves the machine pointing at a dead port.
- macOS app starts containers marked auto-start, and can launch at login.
- Turkish README, security policy, contributing guide.

### Fixed

- Every preset now splits inside the hostname. Three of the four never did.
- HTTP `Host:` fragmentation never ran. It does now.
- Chaining two splitting transforms reordered the byte stream and corrupted the payload.
- The padding, decoy and header transforms appended or overwrote bytes in TCP streams,
  breaking TLS handshakes. They only ever made sense on raw packets and are gone.
- The control daemon stopped answering after ten commands.
- `SetConfig` validated a config and then threw it away.
- `Start` ignored the listen address it was given.
- A second request on a kept-alive proxy connection went to the first request's upstream.
- A CONNECT line split across reads was mis-parsed. Request heads are buffered now.
- DoH cache grew without limit, ignored record TTLs, asked only for A records and only
  ever tried the first address.
- The SOCKS5 backend resolved through the system resolver, which is the poisoned one.
- IPv6 targets in bracket form failed to parse.
- Windows: restarting the machine with protection on left the system proxy pointing at a
  dead port, so nothing on the machine reached the internet until TurkeyDPI was opened
  again. The tray now hands the connection back on shutdown, clears a proxy left behind
  by an earlier run on startup, and registers a logon cleanup for the times it is killed
  outright.
- Windows: the tray kept the system proxy pointing at itself even when the proxy failed
  to start, for instance when the port was already taken.
- macOS: force-quitting left the system proxy pointing at a dead port.
- macOS: killed every TurkeyDPI process on the machine, not its own.
- `config.example.toml` documented eleven keys the schema never had. Unknown keys are now
  rejected instead of ignored.

### Changed

- Relays use `copy_bidirectional`; buffers halved and connections are capped.
- No fallback to the system resolver when DoH fails.
- The unimplemented TUN backend is gone.
- CI runs fmt, clippy at `-D warnings`, and tests on Linux, macOS and Windows.

## 0.1.0

First release.
