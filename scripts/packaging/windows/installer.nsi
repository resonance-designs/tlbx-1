!include "MUI2.nsh"

!define PRODUCT_NAME "GrainRust"
!define PRODUCT_VERSION "0.1.0"
!define COMPANY_NAME "GrainRust"

!define STANDALONE_SRC "dist\\windows\\standalone\\grainrust.exe"
!define VST3_SRC "dist\\windows\\vst3\\GrainRust.vst3"

OutFile "dist\\windows\\GrainRust-Setup.exe"
InstallDir "$PROGRAMFILES\\GrainRust"
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
  CreateShortCut "$DESKTOP\\GrainRust.lnk" "$INSTDIR\\grainrust.exe"
SectionEnd

Section "VST3 Plugin" SecVST3
  SetOutPath "$COMMONFILES\\VST3"
  File /r "${VST3_SRC}"
SectionEnd

Section "Uninstall"
  Delete "$DESKTOP\\GrainRust.lnk"
  Delete "$INSTDIR\\grainrust.exe"
  RMDir "$INSTDIR"
  RMDir /r "$COMMONFILES\\VST3\\GrainRust.vst3"
SectionEnd
