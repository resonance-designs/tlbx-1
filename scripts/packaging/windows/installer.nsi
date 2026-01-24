!include "MUI2.nsh"

!define PRODUCT_NAME "TLBX-1"
!define PRODUCT_VERSION "0.1.15"
!define COMPANY_NAME "TLBX-1"

!define STANDALONE_SRC "dist\\windows\\standalone\\tlbx-1.exe"
!define VST3_SRC "dist\\windows\\vst3\\TLBX-1.vst3"

OutFile "dist\\windows\\TLBX-1-Setup.exe"
InstallDir "$PROGRAMFILES\\TLBX-1"
RequestExecutionLevel admin

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

Section "Standalone App" SecStandalone
  SectionIn RO
  SetOutPath "$INSTDIR"
  File "${STANDALONE_SRC}"
  SetOutPath "$INSTDIR\\documentation"
  File /r "dist\\windows\\documentation\\*"
  CreateShortCut "$DESKTOP\\TLBX-1.lnk" "$INSTDIR\\tlbx-1.exe"
SectionEnd

Section "VST3 Plugin" SecVST3
  SetOutPath "$COMMONFILES\\VST3"
  File /r "${VST3_SRC}"
SectionEnd

Section "Uninstall"
  Delete "$DESKTOP\\TLBX-1.lnk"
  Delete "$INSTDIR\\tlbx-1.exe"
  RMDir "$INSTDIR"
  RMDir /r "$COMMONFILES\\VST3\\TLBX-1.vst3"
SectionEnd
