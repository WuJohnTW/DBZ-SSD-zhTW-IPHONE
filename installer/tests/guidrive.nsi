; GUI driver for run_wine_tests.py --gui: a silent NSIS exe that finds a window of the installer under Wine and
; screenshots it, clicks a button, sets a field's text, lists its controls or reads a message box's text.
;
;   guidrive.exe <args file>
;
; The args file is UTF-16LE, one KEY=value per line: CMD, TITLE, KIND (main|box), ID, TEXT, OUT, LINE, MODE.
; A file rather than /KEY=value options: Wine (like PowerShell) quotes a whole argument that contains a space, and
; GetOptions cannot read "/TITLE=… …".
;
;   wait      wait (up to 20 s) until the window exists and is visible
;   shot      save the window (frame included) as a 24-bit BMP at OUT; MODE print|blt|screen picks how it is copied
;   click     post BN_CLICKED for control ID to its parent (posted, so a message box opened by it does not block us)
;   settext   WM_SETTEXT TEXT on control ID
;   check     BM_SETCHECK checked on control ID (uncheck: unchecked)
;   treecheck on the tree view ID, select root item number TEXT (0-based) and press Space (toggles its checkbox)
;   dump      write "<id>\t<class>\t<state>\t<text>" for every descendant control to OUT (state: visible=, enabled=,
;             rect=<left>,<top>,<width>,<height> in screen pixels)
;   boxtext   write the text of a message box (static 0xFFFF) to OUT
;   fits      for every shown text (static SS_LEFT/CENTER/RIGHT, check box or radio button with text) write "<id>\t<need>\t<height>\t<bottom>\t
;             <parent height>" to OUT: need = the height the installer measured for its text in its own font (window
;             property 0x5A11 that ui/skin.nsh sets; 0 = never measured); bottom and parent height in the parent's client
;             area. (Wine cannot read another process's font, so guidrive cannot measure it itself)
;             and for every shown control "C\t<id>\t<class>\t<style>\t<left>\t<top>\t<right>\t<bottom>\t<parent width>\t
;             <parent height>\t<screen left>\t<top>\t<right>\t<bottom>" (left… in the parent's client area)
;   exec      run the command line LINE verbatim (no re-quoting); exit with its exit code
;   hold      open the file OUT and keep it open TEXT ms (a file in use); MODE = share mode, default 1 (read only)
;
; TITLE: the window title starts with it (NSIS appends the page's subcaption). KIND=main is the wizard (has control
; 1018), box is a message box (has the static 0xFFFF); both are #32770 with the same title. ID: the first visible
; descendant control with that ID. Exit code 0 = done, 1 = window or control not found, 2 = no args file.
; Build: makensis -DOUTFILE=<exe> guidrive.nsi (run_wine_tests.py does it).

Unicode true
SilentInstall silent
RequestExecutionLevel user
; DPI-aware like the installer: otherwise Wine gives this tool sizes scaled to 96 DPI, not the installer's pixels
ManifestDPIAware true
!ifndef OUTFILE
  !define OUTFILE "guidrive.exe"
!endif
OutFile "${OUTFILE}"

!include "LogicLib.nsh"
!include "FileFunc.nsh"
!include "WinMessages.nsh"

Var Cmd
Var Title
Var Kind
Var Id
Var Text
Var Out
Var Hwnd
Var Ctl
Var Line
Var Mode

; $Title／$Kind → $Hwnd（0 = not found）. The title only has to start with $Title: NSIS appends each page's
; subcaption (": 安裝資料夾" and so on)
Function FindTop
  StrCpy $Hwnd 0
  StrCpy $0 0
  StrLen $7 $Title
  ${Do}
    System::Call 'user32::FindWindowExW(p 0, p r0, w "#32770", p 0) p .r0'
    ${If} $0 P= 0
      ${ExitDo}
    ${EndIf}
    System::Call 'user32::IsWindowVisible(p r0) i .r1'
    ${If} $1 = 0
      ${Continue}
    ${EndIf}
    System::Call 'user32::GetWindowTextW(p r0, w .r1, i ${NSIS_MAX_STRLEN}) i'
    StrCpy $1 $1 $7
    ${If} $1 != $Title
      ${Continue}
    ${EndIf}
    ${If} $Kind == "box"
      System::Call 'user32::GetDlgItem(p r0, i 0xFFFF) p .r1'
    ${ElseIf} $Kind == "any"
      StrCpy $1 1   ; any visible #32770 with the title (e.g. the folder browser)
    ${Else}
      System::Call 'user32::GetDlgItem(p r0, i 1018) p .r1'
    ${EndIf}
    ${If} $1 P<> 0
      StrCpy $Hwnd $0
      ${ExitDo}
    ${EndIf}
  ${Loop}
FunctionEnd

; $Hwnd／$Id → $Ctl (0 = not found): the first visible descendant with that control ID, depth-first. MUI pages live
; in a child dialog whose own ID is 0 (control 1018 is only a hidden placeholder), so a path of IDs does not work
Function FindCtl
  StrCpy $Ctl 0
  Push $Hwnd
  Call FindCtlIn
FunctionEnd

Function FindCtlIn
  Exch $0
  Push $1
  Push $2
  Push $3
  System::Call 'user32::GetWindow(p r0, i 5) p .r1'   ; GW_CHILD
  ${Do}
    ${If} $1 P= 0
    ${OrIf} $Ctl P<> 0
      ${ExitDo}
    ${EndIf}
    System::Call 'user32::GetDlgCtrlID(p r1) i .r2'
    System::Call 'user32::IsWindowVisible(p r1) i .r3'
    ${If} $2 = $Id
    ${AndIf} $3 <> 0
      StrCpy $Ctl $1
    ${Else}
      Push $1
      Call FindCtlIn
    ${EndIf}
    System::Call 'user32::GetWindow(p r1, i 2) p .r1'   ; GW_HWNDNEXT
  ${Loop}
  Pop $3
  Pop $2
  Pop $1
  Pop $0
FunctionEnd

; write descendants of $0 to file handle $9 (depth-first)
Function DumpChildren
  Push $0
  Push $1
  Push $2
  Push $3
  System::Call 'user32::GetWindow(p r0, i 5) p .r1'   ; GW_CHILD
  ${DoWhile} $1 P<> 0
    System::Call 'user32::GetDlgCtrlID(p r1) i .r2'
    System::Call 'user32::GetClassNameW(p r1, w .r3, i 256) i'
    ; WM_GETTEXT, not GetWindowText: GetWindowText cannot read an edit field of another process
    System::Call 'user32::SendMessageW(p r1, i 0x000D, p ${NSIS_MAX_STRLEN}, w .r4) p'
    System::Call 'user32::IsWindowVisible(p r1) i .r5'
    System::Call 'user32::IsWindowEnabled(p r1) i .r6'
    System::Call '*(i,i,i,i) p .r7'
    System::Call 'user32::GetWindowRect(p r1, p r7) i'
    System::Call '*$7(i .r8, i .r2, i .r5, i .r6)'
    System::Free $7
    IntOp $5 $5 - $8
    IntOp $6 $6 - $2
    StrCpy $7 "$8,$2,$5,$6"
    System::Call 'user32::GetDlgCtrlID(p r1) i .r2'
    System::Call 'user32::IsWindowVisible(p r1) i .r5'
    System::Call 'user32::IsWindowEnabled(p r1) i .r6'
    FileWriteUTF16LE $9 "$2$\t$3$\tvisible=$5 enabled=$6 rect=$7$\t$4$\r$\n"
    Push $1
    Pop $0
    Call DumpChildren
    System::Call 'user32::GetWindow(p r1, i 2) p .r1'   ; GW_HWNDNEXT
  ${Loop}
  Pop $3
  Pop $2
  Pop $1
  Pop $0
FunctionEnd

; for every descendant of $0: shown texts → line in file handle $9 (see "fits")
Function FitsChildren
  Push $0
  Push $1
  Push $2
  Push $3
  Push $4
  Push $5
  Push $6
  Push $7
  Push $8
  System::Call 'user32::GetWindow(p r0, i 5) p .r1'   ; GW_CHILD
  ${DoWhile} $1 P<> 0
    System::Call 'user32::GetClassNameW(p r1, w .r3, i 256) i'
    System::Call 'user32::GetWindowLongW(p r1, i -16) i .r5'
    IntOp $6 $5 & 0x1F
    System::Call 'user32::IsWindowVisible(p r1) i .r7'
    System::Call 'user32::SendMessageW(p r1, i 0x000D, p ${NSIS_MAX_STRLEN}, w .r4) p'
    ${If} $7 <> 0
      ; every shown control: its place in its parent and on the screen
      System::Call '*(i,i,i,i) p .r8'
      System::Call 'user32::GetWindowRect(p r1, p r8) i'
      System::Call '*$8(i .R0, i .R1, i .R2, i .R3)'
      System::Call 'user32::GetParent(p r1) p .r0'
      System::Call 'user32::MapWindowPoints(p 0, p r0, p r8, i 2) i'
      System::Call '*$8(i .R4, i .R5, i .R6, i .R7)'
      System::Call 'user32::GetClientRect(p r0, p r8) i'
      System::Call '*$8(i, i, i .R8, i .R9)'
      System::Free $8
      System::Call 'user32::GetDlgCtrlID(p r1) i .r2'
      FileWriteUTF16LE $9 "C$\t$2$\t$3$\t$5$\t$R4$\t$R5$\t$R6$\t$R7$\t$R8$\t$R9$\t$R0$\t$R1$\t$R2$\t$R3$\r$\n"
    ${EndIf}
    ; texts: statics SS_LEFT/CENTER/RIGHT, and check boxes / radio buttons (their text wraps and can be cut off too)
    StrCpy $8 0
    ${If} $3 == "Static"
    ${AndIf} $6 <= 2
      StrCpy $8 1
    ${ElseIf} $3 == "Button"
      IntOp $6 $5 & 0xF
      ${If} $6 >= 2       ; BS_CHECKBOX, BS_AUTOCHECKBOX, BS_RADIOBUTTON, BS_3STATE, BS_AUTO3STATE
      ${AndIf} $6 <= 6
        StrCpy $8 1
      ${ElseIf} $6 == 9   ; BS_AUTORADIOBUTTON
        StrCpy $8 1
      ${EndIf}
    ${EndIf}
    ${If} $8 == 1
    ${AndIf} $7 <> 0
    ${AndIf} $4 != ""
      ; the height the installer measured for the text (property 0x5A11, ui/skin.nsh SKIN_NEED_PROP; 0 = not measured)
      System::Call 'user32::GetPropW(p r1, p 0x5A11) p .r6'
      System::Call '*(i,i,i,i) p .r8'
      System::Call 'user32::GetClientRect(p r1, p r8) i'
      System::Call '*$8(i, i, i, i .r7)'
      System::Call 'user32::GetParent(p r1) p .r0'
      System::Call 'user32::GetWindowRect(p r1, p r8) i'
      System::Call 'user32::MapWindowPoints(p 0, p r0, p r8, i 2) i'
      System::Call '*$8(i, i, i, i .r4)'
      System::Call 'user32::GetClientRect(p r0, p r8) i'
      System::Call '*$8(i, i, i, i .r5)'
      System::Free $8
      System::Call 'user32::GetDlgCtrlID(p r1) i .r2'
      FileWriteUTF16LE $9 "$2$\t$6$\t$7$\t$4$\t$5$\r$\n"
    ${EndIf}
    Push $1
    Pop $0
    Call FitsChildren
    System::Call 'user32::GetWindow(p r1, i 2) p .r1'   ; GW_HWNDNEXT
  ${Loop}
  Pop $8
  Pop $7
  Pop $6
  Pop $5
  Pop $4
  Pop $3
  Pop $2
  Pop $1
  Pop $0
FunctionEnd

Function Shot
  ; RECT → width/height
  System::Call '*(i,i,i,i) p .r5'
  System::Call 'user32::GetWindowRect(p $Hwnd, p r5) i'
  System::Call '*$5(i .r6, i .r7, i .r8, i .r9)'
  System::Free $5
  IntOp $R0 $8 - $6
  IntOp $R1 $9 - $7
  ; stride = (w*3+3) & ~3, size = stride*h
  IntOp $R2 $R0 * 3
  IntOp $R2 $R2 + 3
  IntOp $R2 $R2 & 0xFFFFFFFC
  IntOp $R3 $R2 * $R1
  IntOp $R4 $R3 + 54
  ; BITMAPFILEHEADER (14) + BITMAPINFOHEADER (40), bottom-up 24 bpp
  System::Call '*(&i2 0x4D42, i R4, i 0, i 54, i 40, i R0, i R1, &i2 1, &i2 24, i 0, i R3, i 2835, i 2835, i 0, i 0) p .R5'
  IntOp $R6 $R5 + 14
  System::Call 'user32::GetWindowDC(p $Hwnd) p .R7'
  System::Call 'gdi32::CreateCompatibleDC(p R7) p .R8'
  System::Call 'gdi32::CreateDIBSection(p R7, p R6, i 0, *p .R9, p 0, i 0) p .r1'
  System::Call 'gdi32::SelectObject(p R8, p r1) p .r2'
  ; MODE=print: PrintWindow (the window paints itself); blt: copy from the window's DC; screen: copy that rectangle
  ; from the desktop DC (Wine's virtual desktop)
  ${If} $Mode == "print"
    System::Call 'user32::PrintWindow(p $Hwnd, p R8, i 2) i .r3'
  ${ElseIf} $Mode == "screen"
    System::Call 'user32::GetDC(p 0) p .r3'
    System::Call 'gdi32::BitBlt(p R8, i 0, i 0, i R0, i R1, p r3, i r6, i r7, i 0x00CC0020) i'
    System::Call 'user32::ReleaseDC(p 0, p r3) i'
  ${Else}
    System::Call 'gdi32::BitBlt(p R8, i 0, i 0, i R0, i R1, p R7, i 0, i 0, i 0x00CC0020) i'
  ${EndIf}
  System::Call 'gdi32::GdiFlush() i'
  System::Call 'kernel32::CreateFileW(w "$Out", i 0x40000000, i 0, p 0, i 2, i 0x80, p 0) p .r4'
  System::Call 'kernel32::WriteFile(p r4, p R5, i 54, *i .r6, p 0) i'
  System::Call 'kernel32::WriteFile(p r4, p R9, i R3, *i .r6, p 0) i'
  System::Call 'kernel32::CloseHandle(p r4) i'
  System::Call 'gdi32::SelectObject(p R8, p r2) p'
  System::Call 'gdi32::DeleteObject(p r1) i'
  System::Call 'gdi32::DeleteDC(p R8) i'
  System::Call 'user32::ReleaseDC(p $Hwnd, p R7) i'
  System::Free $R5
FunctionEnd

Section
  ; the only parameter is the args file (GetParameters keeps the quotes Wine adds around a path with spaces)
  ${GetParameters} $R9
  StrCpy $0 $R9 1
  ${If} $0 == "$\""
    StrCpy $R9 $R9 -1 1
  ${EndIf}
  ClearErrors
  FileOpen $9 "$R9" r
  ${If} ${Errors}
    SetErrorLevel 2
    Quit
  ${EndIf}
  FileReadWord $9 $0
  ${Do}
    ClearErrors
    FileReadUTF16LE $9 $0
    ${If} ${Errors}
      ${ExitDo}
    ${EndIf}
    ; drop the line break
    ${Do}
      StrCpy $1 $0 1 -1
      ${If} $1 != "$\r"
      ${AndIf} $1 != "$\n"
        ${ExitDo}
      ${EndIf}
      StrCpy $0 $0 -1
    ${Loop}
    StrCpy $1 $0 4
    StrCpy $2 $0 "" 4
    StrCpy $3 $0 5
    StrCpy $4 $0 "" 5
    StrCpy $5 $0 6
    StrCpy $6 $0 "" 6
    ${If} $1 S== "CMD="
      StrCpy $Cmd $2
    ${ElseIf} $3 S== "KIND="
      StrCpy $Kind $4
    ${ElseIf} $1 S== "OUT="
      StrCpy $Out $2
    ${ElseIf} $3 S== "TEXT="
      StrCpy $Text $4
    ${ElseIf} $3 S== "LINE="
      StrCpy $Line $4
    ${ElseIf} $3 S== "MODE="
      StrCpy $Mode $4
    ${ElseIf} $5 S== "TITLE="
      StrCpy $Title $6
    ${Else}
      StrCpy $1 $0 3
      ${If} $1 S== "ID="
        StrCpy $Id $0 "" 3
      ${EndIf}
    ${EndIf}
  ${Loop}
  FileClose $9

  ${If} $Cmd == "hold"
    ; open OUT for reading without FILE_SHARE_DELETE / FILE_SHARE_WRITE and keep it open TEXT ms: it cannot be deleted
    ; MODE: the share mode (default 1 = read only; 3 = read and write, still no delete)
    ${If} $Mode == ""
      StrCpy $Mode 1
    ${EndIf}
    System::Call 'kernel32::CreateFileW(w "$Out", i 0x80000000, i $Mode, p 0, i 3, i 0x80, p 0) p .r0'
    ${If} $0 P= -1
      SetErrorLevel 1
      Quit
    ${EndIf}
    Sleep $Text
    System::Call 'kernel32::CloseHandle(p r0) i'
    Quit
  ${EndIf}

  ${If} $Cmd == "exec"
    ClearErrors
    ExecWait $Line $0
    ${If} ${Errors}
      StrCpy $0 255
    ${EndIf}
    SetErrorLevel $0
    Quit
  ${EndIf}

  ; every other command first waits for its window (up to 20 s)
  StrCpy $8 0
  ${Do}
    Call FindTop
    ${If} $Hwnd P<> 0
    ${OrIf} $8 >= 100
      ${ExitDo}
    ${EndIf}
    Sleep 200
    IntOp $8 $8 + 1
  ${Loop}
  ${If} $Hwnd P= 0
    SetErrorLevel 1
    Quit
  ${EndIf}

  ${If} $Cmd == "wait"
    Sleep 300
  ${ElseIf} $Cmd == "shot"
    System::Call 'user32::UpdateWindow(p $Hwnd) i'
    Sleep 400
    Call Shot
  ${ElseIf} $Cmd == "dump"
    FileOpen $9 "$Out" w
    FileWriteWord $9 0xFEFF
    StrCpy $0 $Hwnd
    Call DumpChildren
    FileClose $9
  ${ElseIf} $Cmd == "fits"
    FileOpen $9 "$Out" w
    FileWriteWord $9 0xFEFF
    StrCpy $0 $Hwnd
    Call FitsChildren
    FileClose $9
  ${ElseIf} $Cmd == "boxtext"
    System::Call 'user32::GetDlgItemTextW(p $Hwnd, i 0xFFFF, w .r1, i ${NSIS_MAX_STRLEN}) i'
    FileOpen $9 "$Out" w
    FileWriteWord $9 0xFEFF
    FileWriteUTF16LE $9 "$1"
    FileClose $9
  ${Else}
    Call FindCtl
    ${If} $Ctl P= 0
      SetErrorLevel 1
      Quit
    ${EndIf}
    ${If} $Cmd == "click"
      System::Call 'user32::GetParent(p $Ctl) p .r1'
      System::Call 'user32::GetDlgCtrlID(p $Ctl) i .r2'
      System::Call 'user32::PostMessageW(p r1, i 0x111, p r2, p $Ctl) i'
    ${ElseIf} $Cmd == "settext"
      SendMessage $Ctl ${WM_SETTEXT} 0 "STR:$Text"
    ${ElseIf} $Cmd == "check"
      SendMessage $Ctl ${BM_SETCHECK} 1 0
    ${ElseIf} $Cmd == "uncheck"
      SendMessage $Ctl ${BM_SETCHECK} 0 0
    ${ElseIf} $Cmd == "treecheck"
      ; components page tree: select root item number /TEXT (0-based) and press Space, which NSIS's tree toggles
      SendMessage $Ctl 0x110A 0 0 $1           ; TVM_GETNEXTITEM, TVGN_ROOT
      StrCpy $2 0
      ${DoWhile} $2 < $Text
        SendMessage $Ctl 0x110A 1 $1 $1        ; TVGN_NEXT
        IntOp $2 $2 + 1
      ${Loop}
      SendMessage $Ctl 0x110B 9 $1             ; TVM_SELECTITEM, TVGN_CARET
      System::Call 'user32::SetFocus(p $Ctl) p'
      System::Call 'user32::PostMessageW(p $Ctl, i 0x100, p 0x20, p 0x00390001) i'   ; WM_KEYDOWN VK_SPACE
      System::Call 'user32::PostMessageW(p $Ctl, i 0x101, p 0x20, p 0xC0390001) i'   ; WM_KEYUP
    ${EndIf}
    Sleep 300
  ${EndIf}
SectionEnd
