# Tauri NSIS Packaging

These assets package a pinned EasyTier runtime into a Windows Tauri application
without putting private-network credentials in the installer.

## Build Inputs

Fetch the matching runtime before building the installer:

~~~powershell
pwsh -NoProfile -File .\scripts\Fetch-EasyTierRuntime.ps1 -Architecture x64
pwsh -NoProfile -File .\scripts\Fetch-Iperf3Runtime.ps1 -Architecture x64
cargo build --release --package vibe-easytier-service --target x86_64-pc-windows-msvc
pwsh -NoProfile -File .\scripts\Stage-VibeEasyTierService.ps1 -Architecture x64
pwsh -NoProfile -File .\scripts\Test-EasyTierPackaging.ps1 -Architecture x64 -RequireRuntime -RequireServiceBinary
~~~

The v1 installer target is Windows x64. Pinned asset names, release tags,
sizes, and SHA-256 values live in the EasyTier and iperf3 runtime manifests.

## Tauri Configuration

Merge exactly one architecture-specific fragment into the application's
src-tauri/tauri.conf.json:

- installer/tauri-nsis.x64.fragment.json
- installer/tauri-nsis.arm64.fragment.json

The relative paths in each fragment are written from src-tauri. Do not use the
fragment as a standalone configuration file because it intentionally omits the
product metadata, build settings, and application capabilities.

The resulting resource layout is:

~~~text
$INSTDIR\resources\easytier\easytier-core.exe
$INSTDIR\resources\iperf3\iperf3.exe
$INSTDIR\resources\iperf3\cygwin1.dll
$INSTDIR\resources\service\vibe-easytier-service.exe
$INSTDIR\resources\scripts\Register-EasyTierService.ps1
$INSTDIR\resources\scripts\Unregister-EasyTierService.ps1
$INSTDIR\resources\scripts\Remove-VibeEasyTierState.ps1
~~~

installMode: perMachine is required. The service runs as LocalSystem, uses
Windows delayed automatic start, and needs an elevated installer and an
elevated configuration writer.

`tauri-nsis.template.nsi` is intentionally pinned to Tauri CLI 2.11.4. It
forces existing Vibe NSIS installs to update in place before the generic Tauri
maintenance page can offer an old-full-uninstaller path. That preserves the
boot service and encrypted desired state if later installer steps are
cancelled or fail. PREINSTALL then stops the existing service and POSTINSTALL
updates it after the new files are present. Update the template and CLI version
together rather than replacing it with an arbitrary Tauri release.

## Service Lifecycle

The bundled NSIS hooks stop the existing VibeEasyTierService before an upgrade
so its core process can be replaced, but retain the SCM registration until
POSTINSTALL updates it. That preserves the established interactive pipe owner
even when a different administrator performs the upgrade. A real uninstall
removes the service. A first installation starts with an empty desired state; it
is still registered at boot so the service is durable before the first
private-network profile is created.

The installer invokes Register-EasyTierService.ps1 with the installed service
binary, the installed EasyTier and iperf3 runtime directories, and the
ProgramData state directory. The script creates a System-and-Administrators-only
state directory, configures delayed start plus recovery actions, and starts the
supervisor with `--service`, `--state-root`, `--core`, and `--iperf3` arguments.
The supervisor, not SCM or the desktop UI, owns `easytier-core.exe`, the
generated runtime TOML, and the virtual-IP-bound iperf3 server.

Registration also creates an `iperf3.exe`-scoped inbound TCP 29999 firewall
rule used by node-to-node bandwidth tests. iperf3 binds only to the active
EasyTier virtual IPv4 address. Upgrades replace the rule in place; a real
uninstall removes it, while `-KeepRegistration` preserves it during the
PREINSTALL stop window.

The uninstaller removes the service, then deletes the protected
%ProgramData%\VibeEasyTier state directory. Both Tauri `/UPDATE` and an
interactive upgrade remain in place until PREINSTALL stops the old service and
keeps its SCM registration. POSTINSTALL updates that registration after the new
files are in place. A normal user-initiated uninstall is the only path that
executes `NSIS_HOOK_PREUNINSTALL`, so it removes the service and protected
state even though NSIS may add its internal `_?=` self-copy argument.

## Verification

Run `Test-EasyTierService.ps1` with the installed service binary, EasyTier and
iperf3 runtime directories, the ProgramData state directory, and
`-RequireRunning`. Packaging validation also runs real loopback iperf3 upload
and reverse-download probes before creating the installer.
