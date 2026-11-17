; Beebeeb's installer hooks for the native app (0.9.0 and later).
; The installer's pages and words are the NSIS template's own, as in 0.8.x: "Installing", "Please wait while Beebeeb
; is being installed.", "Completing Beebeeb Setup", "Beebeeb has been installed on your computer.", "Click Finish to
; close Setup." and the ticked "Run Beebeeb" box. Beebeeb supplies only its name and icon. A per-user install: no
; administrator, no UAC prompt. An update runs the same installer by itself, showing only its progress (/P).

!macro NSIS_HOOK_PREINSTALL
!macroend

; Beebeeb: after the files are in place, remove what only the web-view app used. Only these exact folders, inside
; the app's own data folders; state.db, the device identity and the lock file beside them are never touched.
;   %LOCALAPPDATA%\<identifier>\EBWebView   WebView2's profile (cache, cookies, local storage) of the 0.8.x windows.
; A folder still held by a closing WebView2 process is left; the next install removes it. Nothing reads it again.
!macro NSIS_HOOK_POSTINSTALL
  RMDir /r "$LOCALAPPDATA\${BUNDLEID}\EBWebView"
  ; Beebeeb: the helpers start in a less-privileged AppContainer (opted out of "all application packages"). Let "all
  ; restricted application packages" (S-1-15-2-2) read and run the two helpers, so the container can load its own exe
  ; from a per-user folder whatever the folder's inherited ACL is. These two files only, read and execute, nothing
  ; writable. (Measured on the Windows 11 23H2 build VM: the Office helper also starts from an ungranted copy, so
  ; this is belt and braces there, not the cure for any failure.)
  nsExec::Exec 'icacls "$INSTDIR\beebeeb-office-render.exe" /grant *S-1-15-2-2:(RX)'
  Pop $0
  nsExec::Exec 'icacls "$INSTDIR\beebeeb-media-helper.exe" /grant *S-1-15-2-2:(RX)'
  Pop $0
  ; ...and traverse the folder to reach them.
  nsExec::Exec 'icacls "$INSTDIR" /grant *S-1-15-2-2:(RX)'
  Pop $0
!macroend

!macro NSIS_HOOK_PREUNINSTALL
!macroend

; Beebeeb: an uninstall (not an update) also removes downloaded update installers. They are a cache.
!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
    RMDir /r "$LOCALAPPDATA\${BUNDLEID}\updates"
  ${EndIf}
!macroend
