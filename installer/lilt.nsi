; Lilt installer (NSIS 3).
;
; Build from the repository root after `cargo build --release`:
;   makensis /DVERSION=0.1.0 installer\lilt.nsi
;
; Installs per user (no administrator rights), so the installer, Lilt and its start-up
; entry all live under the current user's profile and registry hive.

Unicode true
SetCompressor /SOLID lzma
ManifestDPIAware true
RequestExecutionLevel user

!ifndef VERSION
  !error "Pass the version with /DVERSION=x.y.z"
!endif

!define APP_NAME "Lilt"
!define PUBLISHER "Shreyan Bag"
!define WEBSITE "https://github.com/shrenixcodes/Lilt"
!define EXE "lilt.exe"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Lilt"
!define RUN_KEY "Software\Microsoft\Windows\CurrentVersion\Run"
!define ROOT ".."

!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "WinVer.nsh"
!include "x64.nsh"
!include "FileFunc.nsh"
!include "WinMessages.nsh"

Name "${APP_NAME}"
OutFile "${ROOT}\target\installer\Lilt-${VERSION}-x64-setup.exe"
InstallDir "$LOCALAPPDATA\Programs\Lilt"
InstallDirRegKey HKCU "${UNINSTALL_KEY}" "InstallLocation"
BrandingText "${APP_NAME} ${VERSION}"

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "${APP_NAME}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "FileDescription" "${APP_NAME} installer"
VIAddVersionKey "CompanyName" "${PUBLISHER}"
VIAddVersionKey "LegalCopyright" "Copyright (c) 2026 ${PUBLISHER}"

!define MUI_ICON "${ROOT}\assets\icons\lilt.ico"
!define MUI_UNICON "${ROOT}\assets\icons\lilt.ico"
!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN "$INSTDIR\${EXE}"
!define MUI_FINISHPAGE_RUN_TEXT "Start Lilt"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "${ROOT}\LICENSE"
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

; Asks a running copy to quit through its control window, then waits for it.
!macro CloseRunningLilt
  FindWindow $0 "Lilt.Control"
  ${If} $0 != 0
    SendMessage $0 ${WM_CLOSE} 0 0 /TIMEOUT=3000
    Sleep 800
  ${EndIf}
!macroend

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "Lilt requires 64-bit Windows."
    Abort
  ${EndIf}
  ${IfNot} ${AtLeastWin10}
    MessageBox MB_ICONSTOP "Lilt requires Windows 10 or later."
    Abort
  ${EndIf}
FunctionEnd

Section "Lilt" SecApp
  SectionIn RO
  !insertmacro CloseRunningLilt

  SetOutPath "$INSTDIR"
  File "${ROOT}\target\release\${EXE}"
  File "/oname=LICENSE.txt" "${ROOT}\LICENSE"
  File "/oname=THIRD-PARTY-NOTICES.md" "${ROOT}\THIRD-PARTY-NOTICES.md"
  WriteUninstaller "$INSTDIR\uninstall.exe"

  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "${APP_NAME}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "Publisher" "${PUBLISHER}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "URLInfoAbout" "${WEBSITE}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\${EXE}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKCU "${UNINSTALL_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "EstimatedSize" $0

  ; An existing start-up entry from a previous installation points at the old path.
  ReadRegStr $0 HKCU "${RUN_KEY}" "Lilt"
  ${If} $0 != ""
    WriteRegStr HKCU "${RUN_KEY}" "Lilt" '"$INSTDIR\${EXE}"'
  ${EndIf}
SectionEnd

Section "Start menu shortcut" SecShortcut
  CreateShortcut "$SMPROGRAMS\Lilt.lnk" "$INSTDIR\${EXE}" "" "$INSTDIR\${EXE}" 0
SectionEnd

Section /o "Start Lilt when I sign in" SecStartup
  WriteRegStr HKCU "${RUN_KEY}" "Lilt" '"$INSTDIR\${EXE}"'
SectionEnd

!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SecApp} "The Lilt application."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecShortcut} "Add Lilt to the Start menu."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecStartup} "Start Lilt automatically. This can be changed later in Lilt's settings."
!insertmacro MUI_FUNCTION_DESCRIPTION_END

Section "Uninstall"
  !insertmacro CloseRunningLilt

  Delete "$INSTDIR\${EXE}"
  Delete "$INSTDIR\LICENSE.txt"
  Delete "$INSTDIR\THIRD-PARTY-NOTICES.md"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"

  Delete "$SMPROGRAMS\Lilt.lnk"
  DeleteRegValue HKCU "${RUN_KEY}" "Lilt"
  DeleteRegKey HKCU "${UNINSTALL_KEY}"

  MessageBox MB_YESNO|MB_ICONQUESTION "Also remove your Lilt settings and logs?" /SD IDNO IDNO keep
    RMDir /r "$APPDATA\Lilt"
    RMDir /r "$LOCALAPPDATA\Lilt"
  keep:
SectionEnd
