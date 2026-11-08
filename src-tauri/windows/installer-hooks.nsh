; Beebeeb's NSIS installer hooks (desktop v2, DW-1901 / DW-1902).
; The installer's pages and words are the NSIS template's own (Tauri's default template): "Installing",
; "Please wait while Beebeeb is being installed.", "Completing Beebeeb Setup", "Beebeeb has been installed on your
; computer.", "Click Finish to close Setup." and the ticked "Run Beebeeb" box. Beebeeb supplies only its name and icon
; (tauri.conf.json > bundle.windows.nsis). A per-user install (installMode currentUser): no administrator, no UAC prompt.
; An update runs the same installer by itself, showing only its progress (plugins.updater.windows.installMode passive).
;
; No hook changes the template's behaviour today; the macros exist so a later step has one marked place to go.
!macro NSIS_HOOK_PREINSTALL
!macroend

!macro NSIS_HOOK_POSTINSTALL
!macroend

!macro NSIS_HOOK_PREUNINSTALL
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
!macroend
