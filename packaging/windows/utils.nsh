; Change shell and registry context based on running
; architecture and chosen install mode.
!macro SetContext
  !if "${INSTALLMODE}" == "currentUser"
    SetShellVarContext current
  !else if "${INSTALLMODE}" == "perMachine"
    SetShellVarContext all
  !endif

  ${If} ${RunningX64}
    !if "${ARCH}" == "x64"
      SetRegView 64
    !else if "${ARCH}" == "arm64"
      SetRegView 64
    !else
      SetRegView 32
    !endif
  ${EndIf}
!macroend

; Beebeeb: the installer never ends a process. (The NSIS template killed every process of the binary's name, without
; asking in passive mode: an upload in progress was cut, and a test build's installer closed the real app.)
;
; If this user runs the app, the installer asks it to quit and waits:
;   1. A native app (0.9.0 and later) is asked through its own single-instance channel: the installed binary is
;      started with --quit-for-update, which hands the request to the running instance; that instance stops the
;      engine, parks its uploads and exits (contract for the shell lane, spec section 5). An update started by the
;      app itself needs no asking: the app is already on its way out and the installer only waits.
;   2. It waits up to 20 seconds for the process to be gone.
;   3. Still running: it says so and offers Retry / Cancel. It does not kill. A silent install (/S) fails with a
;      line on the console. A hung app is ended by the person, in Task Manager; the message says so.
; Only the process of this product's own binary name is looked at; a test build has another name
; (beebeeb-desktop-test.exe), so its installer cannot see, wait for or disturb the real app.
!macro CheckIfAppIsRunning executableName productName
  !define UniqueID ${__LINE__}

  nsis_tauri_utils::StrReplace "$(appRunning)" "{{product_name}}" "${productName}"
  Pop $R1
  StrCpy $R2 0 ; 1 once the app was asked to quit

  wait_${UniqueID}:
  StrCpy $R3 0 ; half seconds waited in this round
  poll_${UniqueID}:
    !if "${INSTALLMODE}" == "currentUser"
      nsis_tauri_utils::FindProcessCurrentUser "${executableName}"
    !else
      nsis_tauri_utils::FindProcess "${executableName}"
    !endif
    Pop $R0
    ; 0 means "found"; anything else: not running.
    ${If} $R0 <> 0
      Goto app_check_done_${UniqueID}
    ${EndIf}
    ${If} $R2 = 0
      StrCpy $R2 1
      ReadRegStr $R4 SHCTX "${UNINSTKEY}" "DisplayVersion"
      ${If} $R4 != ""
        nsis_tauri_utils::SemverCompare "$R4" "0.9.0"
        Pop $R4
        ${If} $R4 = 0
        ${OrIf} $R4 = 1
          ${If} ${FileExists} "$INSTDIR\${executableName}"
            Exec '"$INSTDIR\${executableName}" --quit-for-update'
          ${EndIf}
        ${EndIf}
      ${EndIf}
    ${EndIf}
    ${If} $R3 < 40
      Sleep 500
      IntOp $R3 $R3 + 1
      Goto poll_${UniqueID}
    ${EndIf}

  IfSilent silent_${UniqueID}
  MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "$R1" IDRETRY wait_${UniqueID}
  Abort "$R1"
  silent_${UniqueID}:
    System::Call 'kernel32::AttachConsole(i -1)i.r0'
    ${If} $0 != 0
      System::Call 'kernel32::GetStdHandle(i -11)i.r0'
      System::call 'kernel32::SetConsoleTextAttribute(i r0, i 0x0004)' ; set red color
      FileWrite $0 "$R1$\n"
    ${EndIf}
    Abort
  app_check_done_${UniqueID}:
    !undef UniqueID
!macroend

; Beebeeb: this installer upgrades ${MINUPGRADEFROM} and later only (spec section 2). ${MINUPGRADEFROM} is the bridge
; release: the first one that writes the credential owner tags and runs the pre-flight the native engine relies on.
; An install whose version is lower, empty or not a plain version is refused: unless the version is known to be
; at least the bridge, nothing is replaced. The person is told how to go on, and when an old updater started this
; installer (it has already closed the app), the app they had is started again.
!macro RefuseBelowBridge installedVersion
  StrCpy $R9 -9
  ${If} "${installedVersion}" != ""
    nsis_tauri_utils::SemverCompare "${installedVersion}" "${MINUPGRADEFROM}"
    Pop $R9
  ${EndIf}
  ${If} $R9 <> 0
  ${AndIf} $R9 <> 1
    ${IfNot} ${Silent}
      MessageBox MB_OK|MB_ICONEXCLAMATION "$(needsBridgeFirst)"
    ${EndIf}
    ${If} $UpdateMode = 1
    ${AndIf} ${FileExists} "$INSTDIR\${MAINBINARYNAME}.exe"
      Exec '"$INSTDIR\${MAINBINARYNAME}.exe"'
    ${EndIf}
    SetErrorLevel 3
    Quit
  ${EndIf}
!macroend

; Sets AppUserModelId on a shortcut
!macro SetLnkAppUserModelId shortcut
  !insertmacro ComHlpr_CreateInProcInstance ${CLSID_ShellLink} ${IID_IShellLink} r0 ""
  ${If} $0 P<> 0
    ${IUnknown::QueryInterface} $0 '("${IID_IPersistFile}",.r1)'
    ${If} $1 P<> 0
      ${IPersistFile::Load} $1 '("${shortcut}", ${STGM_READWRITE})'
      ${IUnknown::QueryInterface} $0 '("${IID_IPropertyStore}",.r2)'
      ${If} $2 P<> 0
        System::Call 'Oleaut32::SysAllocString(w "${BUNDLEID}") i.r3'
        System::Call '*${SYSSTRUCT_PROPERTYKEY}(${PKEY_AppUserModel_ID})p.r4'
        System::Call '*${SYSSTRUCT_PROPVARIANT}(${VT_BSTR},,&i4 $3)p.r5'
        ${IPropertyStore::SetValue} $2 '($4,$5)'

        System::Call 'Oleaut32::SysFreeString($3)'
        System::Free $4
        System::Free $5
        ${IPropertyStore::Commit} $2 ""
        ${IUnknown::Release} $2 ""
        ${IPersistFile::Save} $1 '("${shortcut}",1)'
      ${EndIf}
      ${IUnknown::Release} $1 ""
    ${EndIf}
    ${IUnknown::Release} $0 ""
  ${EndIf}
!macroend

; Deletes jump list entries and recent destinations
!macro DeleteAppUserModelId
  !insertmacro ComHlpr_CreateInProcInstance ${CLSID_DestinationList} ${IID_ICustomDestinationList} r1 ""
  ${If} $1 P<> 0
    ${ICustomDestinationList::DeleteList} $1 '("${BUNDLEID}")'
    ${IUnknown::Release} $1 ""
  ${EndIf}
  !insertmacro ComHlpr_CreateInProcInstance ${CLSID_ApplicationDestinations} ${IID_IApplicationDestinations} r1 ""
  ${If} $1 P<> 0
    ${IApplicationDestinations::SetAppID} $1 '("${BUNDLEID}")i.r0'
    ${If} $0 >= 0
      ${IApplicationDestinations::RemoveAllDestinations} $1 ''
    ${EndIf}
    ${IUnknown::Release} $1 ""
  ${EndIf}
!macroend

; Unpins a shortcut from Start menu and Taskbar
;
; From https://stackoverflow.com/a/42816728/16993372
!macro UnpinShortcut shortcut
  !insertmacro ComHlpr_CreateInProcInstance ${CLSID_StartMenuPin} ${IID_IStartMenuPinnedList} r0 ""
  ${If} $0 P<> 0
      System::Call 'SHELL32::SHCreateItemFromParsingName(ws, p0, g "${IID_IShellItem}", *p0r1)' "${shortcut}"
      ${If} $1 P<> 0
          ${IStartMenuPinnedList::RemoveFromList} $0 '(r1)'
          ${IUnknown::Release} $1 ""
      ${EndIf}
      ${IUnknown::Release} $0 ""
  ${EndIf}
!macroend

; Set target path for a .lnk shortcut
!macro SetShortcutTarget shortcut target
  !insertmacro ComHlpr_CreateInProcInstance ${CLSID_ShellLink} ${IID_IShellLink} r0 ""
  ${If} $0 P<> 0
    ${IUnknown::QueryInterface} $0 '("${IID_IPersistFile}",.r1)'
    ${If} $1 P<> 0
      ${IPersistFile::Load} $1 '("${shortcut}", ${STGM_READWRITE})'
      ${IShellLink::SetPath} $0 '(w "${target}")'
      ${IPersistFile::Save} $1 '("${shortcut}",1)'
      ${IUnknown::Release} $1 ""
    ${EndIf}
    ${IUnknown::Release} $0 ""
  ${EndIf}
!macroend

!define /ifndef MAX_PATH 260
!define /ifndef SLGP_RAWPATH 0x4

; Test if a .lnk shortcut's target is target,
; use Pop to get the result, 1 is yes, 0 is no,
; note that this macro modifies $0, $1, $2, $3
;
; Exmaple usage:
;   !insertmacro "IsShortCutTarget" "C:\Users\Public\Desktop\App.lnk" "C:\Program Files\App\App.exe"
;   Pop $0
;   ${If} $0 = 1
;     MessageBox MB_OK "shortcut target matches"
;   ${EndIf}
!macro IsShortcutTarget shortcut target
  ; $0: IShellLink
  ; $1: IPersistFile
  ; $2: Target path
  ; $3: Return value

  StrCpy $3 0
  !insertmacro ComHlpr_CreateInProcInstance ${CLSID_ShellLink} ${IID_IShellLink} r0 ""
  ${If} $0 P<> 0
    ${IUnknown::QueryInterface} $0 '("${IID_IPersistFile}", .r1)'
    ${If} $1 P<> 0
      ${IPersistFile::Load} $1 '("${shortcut}", ${STGM_READ})'
      System::Alloc MAX_PATH
      Pop $2
      ${IShellLink::GetPath} $0 '(.r2, ${MAX_PATH}, 0, ${SLGP_RAWPATH})'
      ${If} $2 == "${target}"
        StrCpy $3 1
      ${EndIf}
      System::Free $2
      ${IUnknown::Release} $1 ""
    ${EndIf}
    ${IUnknown::Release} $0 ""
  ${EndIf}
  Push $3
!macroend
