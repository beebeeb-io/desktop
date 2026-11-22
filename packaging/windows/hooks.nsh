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

; Beebeeb (drive-complete-006b): run by the uninstall section once the app has stopped and while its exe is still
; here. The app unregisters THIS identity's sync folder from Cloud Files and Explorer (navigation-pane entry, status
; flyout and thumbnail handlers), then keeps downloaded files as plain files ("Keep downloaded files" ticked, the
; default, and always in a silent uninstall) or removes the folder (unticked). Engine:
; windows_cf::registration_teardown. An update (/UPDATE) changes nothing here. A failure leaves the uninstall going.
; RC2-7: with "delete … the saved sign-in" ticked, the same run also deletes the saved sign-in (session tokens, wrapped
; keys, account emails, parked sessions): every Credential Manager entry named "<credential service>/…" of THIS
; identity only (engine keychain::forget_sign_in; io.beebeeb.app never matches io.beebeeb.app.test, nor the reverse).
!macro BB_UNINSTALL_SYNC_FOLDER exe
  ${If} $UpdateMode <> 1
  ${AndIf} ${FileExists} "${exe}"
    StrCpy $1 ""
    ${If} $DeleteAppDataCheckboxState = 1
      StrCpy $1 " forget-sign-in"
    ${EndIf}
    ${If} $KeepFilesCheckboxState = ${BST_CHECKED}
      ExecWait '"${exe}" --uninstall-sync-folder keep$1' $0
    ${Else}
      ExecWait '"${exe}" --uninstall-sync-folder remove$1' $0
    ${EndIf}
    DetailPrint "Beebeeb folder teardown: exit $0"
  ${EndIf}
!macroend

; Beebeeb: an uninstall (not an update) also removes downloaded update installers. They are a cache.
; The identity's own names (engine identity.rs): the folder name under the config, data, cache and temp dirs.
; Only THIS identity's: the test build never touches the real app's, nor the other way round.
!if "${BUNDLEID}" == "io.beebeeb.app"
  !define BB_FOLDER "beebeeb"
!else
  !define BB_FOLDER "beebeeb-test"
!endif

!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
    RMDir /r "$LOCALAPPDATA\${BUNDLEID}\updates"
    ${If} $DeleteAppDataCheckboxState = 1
      SetShellVarContext current
      ; settings (desktop.toml) and plaintext staging copies, under the data, cache and temp folders
      RMDir /r "$APPDATA\${BB_FOLDER}"
      RMDir /r "$LOCALAPPDATA\${BB_FOLDER}"
      ; the real app's temp copies sit directly in %TEMP% and cannot be told apart from other programs' files: only the
      ; test build has a folder of its own there
      !if "${BB_FOLDER}" != "beebeeb"
        RMDir /r "$TEMP\${BB_FOLDER}"
      !endif
      ; staging inside the install folder
      RMDir /r "$INSTDIR\finder-writes"
      RMDir "$INSTDIR"
      ; the saved sign-in went earlier, while the app was still here (BB_UNINSTALL_SYNC_FOLDER ... forget-sign-in)
    ${Else}
      ; not ticked: say exactly what stays (silent uninstall shows nothing and never deletes data)
      MessageBox MB_OK|MB_ICONINFORMATION|MB_SETFOREGROUND "$(uninstallKeepsData)" /SD IDOK
    ${EndIf}
  ${EndIf}
!macroend
