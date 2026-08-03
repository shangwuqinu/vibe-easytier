; Tauri NSIS hooks for the Vibe EasyTier Windows service.
; The installer is per-machine, so PowerShell runs elevated and can manage
; the LocalSystem service. Private-network configuration is never bundled.

!macro NSIS_HOOK_PREINSTALL
  ; An upgrade must release the old easytier-core.exe before NSIS overwrites it.
  ; Keep the SCM registration so POSTINSTALL can preserve the original pipe
  ; owner while it updates the service ImagePath. First installs have no script.
  IfFileExists "$INSTDIR\resources\scripts\Unregister-EasyTierService.ps1" easytier_preinstall_unregister easytier_preinstall_done

easytier_preinstall_unregister:
    nsExec::ExecToLog '"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$INSTDIR\resources\scripts\Unregister-EasyTierService.ps1" -ServiceName "VibeEasyTierService" -ExpectedServiceBinaryPath "$INSTDIR\resources\service\vibe-easytier-service.exe" -KeepRegistration'
    Pop $0
    StrCmp $0 0 easytier_preinstall_done
    DetailPrint "Vibe EasyTier service was not stopped before the upgrade (exit $0)."
    Abort "Vibe EasyTier could not stop its existing service. The upgrade was cancelled."

easytier_preinstall_done:
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; Register immediately, including on a first installation with no profile.
  ; The service holds an empty desired state until the desktop client creates
  ; the first encrypted private-network profile.
    nsExec::ExecToLog '"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$INSTDIR\resources\scripts\Register-EasyTierService.ps1" -ServiceBinaryPath "$INSTDIR\resources\service\vibe-easytier-service.exe" -RuntimeDirectory "$INSTDIR\resources\easytier" -Iperf3Directory "$INSTDIR\resources\iperf3" -ServiceName "VibeEasyTierService"'
    Pop $0
    StrCmp $0 0 easytier_postinstall_done
    DetailPrint "Vibe EasyTier service registration failed (exit $0)."
    Abort "Vibe EasyTier could not register its boot service. The installation stopped before the client could be used."

easytier_postinstall_done:
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; A real uninstall removes the service and then its protected ProgramData
  ; state after the supervisor has stopped.
  IfFileExists "$INSTDIR\resources\scripts\Unregister-EasyTierService.ps1" easytier_preuninstall_unregister easytier_preuninstall_missing_unregister

easytier_preuninstall_unregister:
    nsExec::ExecToLog '"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$INSTDIR\resources\scripts\Unregister-EasyTierService.ps1" -ServiceName "VibeEasyTierService" -ExpectedServiceBinaryPath "$INSTDIR\resources\service\vibe-easytier-service.exe"'
    Pop $0
    StrCmp $0 0 easytier_preuninstall_remove_state
    DetailPrint "Vibe EasyTier service cleanup failed (exit $0)."
    Abort "Vibe EasyTier could not remove its service. The uninstall was cancelled."

easytier_preuninstall_remove_state:
  IfFileExists "$INSTDIR\resources\scripts\Remove-VibeEasyTierState.ps1" easytier_preuninstall_remove_state_run easytier_preuninstall_missing_state_cleanup

easytier_preuninstall_remove_state_run:
    nsExec::ExecToLog '"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$INSTDIR\resources\scripts\Remove-VibeEasyTierState.ps1"'
    Pop $0
    StrCmp $0 0 easytier_preuninstall_done
    DetailPrint "Vibe EasyTier state cleanup failed (exit $0)."
    Abort "Vibe EasyTier could not remove its protected state. The uninstall was cancelled."

easytier_preuninstall_missing_unregister:
    Abort "Vibe EasyTier cannot find its service cleanup script. The operation was cancelled to avoid leaving a running boot service behind."

easytier_preuninstall_missing_state_cleanup:
    Abort "Vibe EasyTier cannot find its protected-state cleanup script. The uninstall was cancelled."

easytier_preuninstall_done:
!macroend
