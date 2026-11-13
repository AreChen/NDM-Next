Unicode true
RequestExecutionLevel user

!include "MUI2.nsh"

!ifndef NDM_BIN_DIR
  !error "NDM_BIN_DIR must contain ndm-desktop.exe and ndm-host.exe"
!endif
!ifndef NDM_OUTPUT
  !error "NDM_OUTPUT must be the output installer path"
!endif
!ifndef NDM_NOTICES
  !error "NDM_NOTICES must be the generated third-party licenses HTML"
!endif
!ifndef NDM_VERSION
  !define NDM_VERSION "0.1.0"
!endif
!ifndef NDM_VERSION_QUAD
  !define NDM_VERSION_QUAD "0.1.0.0"
!endif
!ifndef NDM_HOST_NAME
  !define NDM_HOST_NAME "com.ndmnext.host"
!endif
!ifndef NDM_CHROME_ID
  !define NDM_CHROME_ID "caegebfdojcdlkkiaiokeoadefdooamd"
!endif
!ifndef NDM_EDGE_ID
  !ifndef NDM_TEST_INSTALL
    !define NDM_EDGE_ID "hiblommiheggidolemmnnlocmocebckp"
  !endif
!endif

Name "NDM Next"
OutFile "${NDM_OUTPUT}"
InstallDir "$LOCALAPPDATA\Programs\NDM Next"
SetCompressor /SOLID lzma
VIProductVersion "${NDM_VERSION_QUAD}"
VIAddVersionKey "ProductName" "NDM Next"
VIAddVersionKey "CompanyName" "NDM Next contributors"
VIAddVersionKey "FileDescription" "NDM Next installer"
VIAddVersionKey "FileVersion" "${NDM_VERSION}"
VIAddVersionKey "LegalCopyright" "NDM Next contributors"
Icon "ndm-next.ico"
UninstallIcon "ndm-next.ico"

!define MUI_ABORTWARNING
!define MUI_ICON "ndm-next.ico"
!define MUI_UNICON "ndm-next.ico"
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "SimpChinese"
!insertmacro MUI_LANGUAGE "English"

Section "Install"
  SetShellVarContext current
  SetOutPath "$INSTDIR"
  File "${NDM_BIN_DIR}\ndm-desktop.exe"
  File "${NDM_BIN_DIR}\ndm-host.exe"
  File /oname=LICENSE.txt "..\..\..\LICENSE"
  File /oname=THIRD-PARTY-LICENSES.html "${NDM_NOTICES}"
  File /oname=LICENSE-LUCIDE.txt "licenses\LUCIDE.txt"
  WriteUninstaller "$INSTDIR\Uninstall.exe"

  ; The host writes UTF-8 JSON, including escaped non-ASCII install paths.
  !ifdef NDM_EDGE_ID
    nsExec::ExecToStack '"$INSTDIR\ndm-host.exe" --write-manifest "${NDM_HOST_NAME}" "${NDM_CHROME_ID}" "${NDM_EDGE_ID}"'
  !else
    nsExec::ExecToStack '"$INSTDIR\ndm-host.exe" --write-manifest "${NDM_HOST_NAME}" "${NDM_CHROME_ID}"'
  !endif
  Pop $0
  Pop $1
  StrCmp $0 "0" 0 install_error

  WriteRegStr HKCU "Software\Google\Chrome\NativeMessagingHosts\${NDM_HOST_NAME}" "" "$INSTDIR\native-messaging-host.json"
  WriteRegStr HKCU "Software\Chromium\NativeMessagingHosts\${NDM_HOST_NAME}" "" "$INSTDIR\native-messaging-host.json"
  WriteRegStr HKCU "Software\Microsoft\Edge\NativeMessagingHosts\${NDM_HOST_NAME}" "" "$INSTDIR\native-messaging-host.json"

  !ifndef NDM_TEST_INSTALL
    CreateDirectory "$SMPROGRAMS\NDM Next"
    CreateShortcut "$SMPROGRAMS\NDM Next\NDM Next.lnk" "$INSTDIR\ndm-desktop.exe"
    CreateShortcut "$SMPROGRAMS\NDM Next\Uninstall NDM Next.lnk" "$INSTDIR\Uninstall.exe"
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\NDM Next" "DisplayName" "NDM Next"
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\NDM Next" "DisplayVersion" "${NDM_VERSION}"
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\NDM Next" "DisplayIcon" "$INSTDIR\ndm-desktop.exe"
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\NDM Next" "UninstallString" '"$INSTDIR\Uninstall.exe"'
    WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\NDM Next" "NoModify" 1
    WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\NDM Next" "NoRepair" 1
  !endif
  Goto install_done
install_error:
  MessageBox MB_ICONSTOP "Could not create the browser integration manifest."
  Abort
install_done:
SectionEnd

Section "Uninstall"
  SetShellVarContext current
  ; Do not remove a registry entry if another installation replaced it.
  ReadRegStr $0 HKCU "Software\Google\Chrome\NativeMessagingHosts\${NDM_HOST_NAME}" ""
  StrCmp $0 "$INSTDIR\native-messaging-host.json" 0 +2
  DeleteRegKey HKCU "Software\Google\Chrome\NativeMessagingHosts\${NDM_HOST_NAME}"
  ReadRegStr $0 HKCU "Software\Chromium\NativeMessagingHosts\${NDM_HOST_NAME}" ""
  StrCmp $0 "$INSTDIR\native-messaging-host.json" 0 +2
  DeleteRegKey HKCU "Software\Chromium\NativeMessagingHosts\${NDM_HOST_NAME}"
  ReadRegStr $0 HKCU "Software\Microsoft\Edge\NativeMessagingHosts\${NDM_HOST_NAME}" ""
  StrCmp $0 "$INSTDIR\native-messaging-host.json" 0 +2
  DeleteRegKey HKCU "Software\Microsoft\Edge\NativeMessagingHosts\${NDM_HOST_NAME}"
  ReadRegStr $0 HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "NDM Next"
  StrCmp $0 '$\"$INSTDIR\ndm-desktop.exe$\" --background' 0 +2
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "NDM Next"
  !ifndef NDM_TEST_INSTALL
    ReadRegStr $0 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\NDM Next" "UninstallString"
    StrCmp $0 '$\"$INSTDIR\Uninstall.exe$\"' 0 skip_shortcuts
    DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\NDM Next"
    Delete "$SMPROGRAMS\NDM Next\NDM Next.lnk"
    Delete "$SMPROGRAMS\NDM Next\Uninstall NDM Next.lnk"
    RMDir "$SMPROGRAMS\NDM Next"
  skip_shortcuts:
  !endif
  Delete "$INSTDIR\native-messaging-host.json"
  Delete "$INSTDIR\LICENSE.txt"
  Delete "$INSTDIR\THIRD-PARTY-LICENSES.html"
  Delete "$INSTDIR\LICENSE-LUCIDE.txt"
  Delete "$INSTDIR\ndm-host.exe"
  Delete "$INSTDIR\ndm-desktop.exe"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
SectionEnd
