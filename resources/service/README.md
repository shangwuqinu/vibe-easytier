# Vibe EasyTier Service Staging Area

Stage the compiled Windows service host here before building the Tauri NSIS
installer:

~~~powershell
pwsh -NoProfile -File .\scripts\Stage-VibeEasyTierService.ps1 -Architecture x64
~~~

The expected output is:

- windows-x64\vibe-easytier-service.exe
- windows-arm64\vibe-easytier-service.exe

The installer launches this binary as the LocalSystem VibeEasyTierService. It
receives the installed EasyTier runtime directory and the ProgramData state
directory as service arguments, then supervises easytier-core.exe itself.
Generated binaries are ignored by Git.
