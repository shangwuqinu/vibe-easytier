# Vibe EasyTier

Vibe EasyTier is a Windows x64 desktop client for a deliberately small
EasyTier private virtual-LAN workflow. The desktop process owns neither
`easytier-core` nor its management endpoint: `VibeEasyTierService` is the sole
supervisor for the pinned core process.

## Guarantees

- The NSIS installer creates `VibeEasyTierService` as a delayed automatic
  LocalSystem service and configures Windows failure recovery.
- Automatic connection is a durable profile intent. A manual disconnect clears
  that intent before stopping the core, so background recovery does not undo a
  user action. Service start, network recovery, and resume use jittered
  exponential recovery capped at five minutes.
- Profiles use fixed virtual IPv4 CIDRs, a private network name and secret,
  and explicit bootstrap peers. Settings exposes all 41 `EasyTier v2.6.4`
  `[flags]` options with Chinese names and descriptions, including the
  deprecated QUIC listen-port compatibility option. The UI and TOML importer
  still reject non-`[flags]` portal, subnet-routing, proxy, port-forward, and
  configuration-server surfaces. `wg://host:port` is allowed as an EasyTier
  peer transport; it is distinct from the deliberately unsupported WireGuard
  `vpn_portal` server option.
- The service launches the core with `--rpc-portal 127.0.0.1:15888` and a
  loopback whitelist. The desktop UI never calls that port; it uses the
  service-owned named pipe instead.
- Route and traffic cards are sampled from the pinned `easytier-cli` `route
  list` and `stats show` commands. Profile export is requested through the
  protected pipe and written by native Tauri code, so the complete TOML and
  its network secret never enter webview state.
- Node bandwidth tests use bundled iperf3 3.21 clients and a supervised
  iperf3 server. Upload and reverse-download tests run sequentially, with both
  ends bound to their EasyTier virtual IPv4 addresses. Both nodes need this
  Vibe service version; the installer scopes TCP 29999 to `iperf3.exe` in
  Windows Firewall and removes the rule on uninstall. Service-owned executable
  directories allow normal users to read and run binaries but not replace
  LocalSystem child executables.
- Network secrets are encrypted in service-owned state with Windows DPAPI.
  The service writes the active secret only to an ACL-protected runtime TOML,
  keeping it out of the core process command line. The desktop UI gets a
  secret-free profile view through a SID-restricted local named pipe. The
  LocalSystem service owns that pipe and flushes each response before the
  single-request connection is closed.
- Service registration derives the pipe owner from the interactive desktop,
  not an over-the-shoulder UAC account. Uninstall removes both the service and
  the protected `%ProgramData%\VibeEasyTier` state; version upgrades preserve
  that state.

## Local Trust Boundary

EasyTier v2.6.4 has no authenticated per-Windows-SID authorization for its
upstream management RPC. Binding it to loopback keeps it off the LAN, but it
does not fully isolate separate local Windows accounts that can reach
`127.0.0.1`. The supported desktop management path remains the ACL-protected
named pipe. Use a separate Windows instance or upstream RPC authentication for
shared-host account isolation.

## Layout

- `apps/desktop`: React + TypeScript operational UI.
- `src-tauri`: Tauri 2 shell, native tray behavior, and the desktop-to-service
  command boundary.
- `crates/vibe-easytier-service`: persisted desired state, DPAPI envelope,
  ACL helpers, local IPC, profile parsing, and child-process supervisor.
- `scripts` and `installer`: pinned runtime staging plus per-machine NSIS
  registration hooks.

## Local Development

Install Rust stable plus Visual Studio Build Tools with the MSVC desktop C++
workload, then run:

```powershell
npm --prefix .\apps\desktop ci
npm --prefix .\apps\desktop run build
cargo test -p vibe-easytier-service
pwsh -NoProfile -File .\scripts\Fetch-EasyTierRuntime.ps1 -Architecture x64
pwsh -NoProfile -File .\scripts\Fetch-Iperf3Runtime.ps1 -Architecture x64
cargo build --release -p vibe-easytier-service --target x86_64-pc-windows-msvc
pwsh -NoProfile -File .\scripts\Stage-VibeEasyTierService.ps1 -Architecture x64
npm --prefix .\apps\desktop run desktop:dev
```

The native desktop intentionally does not start `easytier-core` itself. For a
real-machine run, install the service through the NSIS package or invoke
`Register-EasyTierService.ps1` from an elevated PowerShell session after the
service executable has been staged.

## Packaging

EasyTier is locked to v2.6.4 and iperf3 is locked to 3.21 in their respective
runtime manifests under `resources`. Validate both inputs before bundling:

```powershell
pwsh -NoProfile -File .\scripts\Test-EasyTierPackaging.ps1 -Architecture x64 -RequireRuntime -RequireServiceBinary -VerifyReleaseMetadata
npm --prefix .\apps\desktop run desktop:build
```

Do not place profile TOML files or private-network secrets under `resources`.
They belong to the protected service state beneath `%ProgramData%\VibeEasyTier`.
