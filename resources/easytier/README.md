# EasyTier Runtime Staging Area

This directory is populated by scripts/Fetch-EasyTierRuntime.ps1.

The fetch script locks the runtime to EasyTier v2.6.4, verifies the pinned
SHA-256 and size, and (unless explicitly skipped) verifies the GitHub release
metadata before extracting the runtime.

Expected generated directories:

- windows-x64
- windows-arm64

Each generated directory contains easytier-core.exe, easytier-cli.exe, and the
DLL/SYS files shipped alongside the selected upstream binary. Runtime payloads
are intentionally ignored by Git; the manifest is the source of truth for
their provenance.

Do not put private-network configuration or credentials in this directory.
VibeEasyTierService receives this installed runtime directory and keeps its
encrypted private-network state plus generated core configuration under
%ProgramData%\VibeEasyTier.
