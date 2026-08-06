# Security

## Threat model

TurkeyDPI defeats one specific thing: a middlebox that reads the hostname out of your
TLS ClientHello or HTTP `Host:` header and drops connections that match a list.

It does not do anything else, and it is worth being blunt about what that means.

**It does not hide you.** Your ISP still sees every IP address you connect to, when, and
how much data you move. Destination IPs are not obscured in any way.

**It is not a VPN or a tunnel.** Nothing is routed through a third party. Traffic goes
directly from your machine to the destination.

**It adds no encryption.** HTTPS is still HTTPS; plaintext HTTP is still plaintext, and
splitting a request across segments does not make it private. Anyone on the path can read
it.

**It does not survive IP-level blocking.** If a site is blocked by address rather than by
hostname, fragmentation changes nothing.

**It does not resist active probing.** An operator who fingerprints unusual segmentation
patterns, or who actively probes destinations, can identify this traffic as evasion. There
is no attempt to look like ordinary traffic.

**DNS-over-HTTPS is visible.** Queries are encrypted, but the connection to the DoH
resolver is not hidden. An operator can see that you use one, and can block it.

## Running it safely

- Keep it bound to loopback. `--listen` accepts any address and there is **no
  authentication of any kind** — binding to `0.0.0.0` gives anyone who can reach the port
  an open proxy. The CLI warns when you do this.
- The system proxy is a machine-wide setting. If TurkeyDPI dies without cleaning up, run
  `turkeydpi unset-proxy` to restore it.
- Setting the system proxy on macOS needs administrator rights; on Windows it writes to
  your own user's registry keys and does not.

## Reporting a vulnerability

Open a [security advisory](https://github.com/1etu/turkeydpi/security/advisories/new), or
an issue if it is not sensitive. Include the version, platform and the steps to reproduce.

This is a hobby project maintained by one person. There is no guaranteed response time.
