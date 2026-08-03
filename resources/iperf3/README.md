# iperf3 Runtime Staging Area

`scripts/Fetch-Iperf3Runtime.ps1` populates `windows-x64` with the pinned
iperf3 3.21 Windows runtime. The script verifies GitHub release metadata,
archive size, and SHA-256 before replacing an existing staged runtime.

The generated payload contains `iperf3.exe`, its required `cygwin1.dll`, and
the tracked third-party notices. Generated runtime files are intentionally
ignored by Git; `resources/iperf3-runtime.manifest.json` is the provenance
source of truth.

The Windows service launches the bundled server only on the active EasyTier
virtual IPv4 address. The desktop launches bundled clients through the native
Tauri boundary. Neither process accepts a user-supplied executable path.
