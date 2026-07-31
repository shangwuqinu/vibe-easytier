# Repository Guidelines

## Project Structure & Module Organization

- `apps/desktop/` contains the React 19 + TypeScript operational UI. Keep UI
  tests next to their modules as `*.test.ts` or `*.test.tsx`.
- `src-tauri/` is the Tauri 2 shell, tray integration, and desktop-to-service
  bridge.
- `crates/vibe-easytier-service/` owns the Windows service, DPAPI state,
  named-pipe IPC, profile validation, and EasyTier Core supervision.
- `resources/` holds pinned, verified runtime assets; `scripts/` stages and
  validates them; `installer/` contains NSIS service hooks. Do not place user
  profiles, TOML files, or secrets in these directories.

## Build, Test, and Development Commands

Run from the repository root in PowerShell:

```powershell
npm --prefix .\apps\desktop ci
npm --prefix .\apps\desktop test -- --run
npm --prefix .\apps\desktop run build
cargo fmt --all -- --check
cargo test --workspace --locked
pwsh -NoProfile -File .\scripts\Test-EasyTierPackaging.ps1 -Architecture x64 -RequireRuntime -RequireServiceBinary -VerifyReleaseMetadata
```

Create a release installer by building and staging the service, then running
`npm --prefix .\apps\desktop run desktop:build`. Use `desktop:dev` only for
interactive development; do not treat a Vite preview as the packaged app.

## Coding Style & Naming Conventions

Use `cargo fmt`; Rust uses four-space indentation, `snake_case` functions and
tests, and `PascalCase` types. TypeScript uses two-space indentation,
`camelCase` values, and `PascalCase` React components. Follow nearby patterns
instead of introducing new abstractions. Keep user-facing UI and errors in
Chinese; retain established technical terms such as `Bootstrap`, `WireGuard`,
and URI schemes. Prefer typed parsing over string manipulation.

## Testing Guidelines

Add a focused regression test with every behavior change. Rust tests live in
module `#[cfg(test)]` blocks; frontend tests use Vitest. Run the full workspace
test suite before packaging. Tests marked `#[ignore]` require the locally
registered Windows service and may create temporary profiles; run them only on
a suitable Windows machine. Validate profile changes with the bundled Core,
not only mocked IPC output.

## Security and Change Reviews

Never log, commit, or render network secrets, staged runtime TOML, DPAPI
payloads, SIDs, or named-pipe credentials. Preserve the service boundary:
the desktop must use local IPC, never the Core RPC portal directly.

This repository has no commit history yet, so use concise imperative messages
such as `feat(peers): show active transports`. Pull requests should explain
the behavior and recovery impact, list commands run, link the issue when one
exists, and include screenshots for visible UI changes.
