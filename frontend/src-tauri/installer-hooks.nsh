; Quickdrop NSIS installer hooks.
;
; The PC acts as a TCP server (port 55432) when RECEIVING files and listens on
; UDP 55433 for discovery. Windows Firewall blocks inbound connections to the
; installed executable by default, which makes the PC unable to receive. Add an
; allow rule for the program on install, and remove it on uninstall. The
; installer runs elevated, so netsh can modify the firewall here.

!macro NSIS_HOOK_POSTINSTALL
  nsExec::ExecToLog 'netsh advfirewall firewall delete rule name="Quickdrop"'
  nsExec::ExecToLog 'netsh advfirewall firewall add rule name="Quickdrop" dir=in action=allow program="$INSTDIR\quickdrop.exe" enable=yes profile=any'
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  nsExec::ExecToLog 'netsh advfirewall firewall delete rule name="Quickdrop"'
!macroend
