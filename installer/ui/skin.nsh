; The look of the installer and the uninstaller, after the original game's dialogue boxes: sky-blue background,
; the cream panel with the red-orange double frame and four corner studs, the ▶ cursor before titles, the portrait frame
; around the Goku art, the dark blue button bar. Only how the pages look: no page logic is here, and nothing
; runs in silent mode (.onGUIInit is not called then).
;
; The pieces are the bitmaps make_ui.py cut from pieces.html (the design's CSS) at 1x and 2x; they are composed at run
; time to each page's real size (dialog sizes follow the system font and DPI): the frames in nine slices (corners as
; they are, edges and middle stretched along their length) with AlphaBlend, never scaled across.
; Texts are not transparent: every text sits on a cream panel and gets the cream background (or the bar's dark blue).
;
; Hooks: MUI_CUSTOMFUNCTION_(UN)GUIINIT → SkinInit (dbz-zhtw.nsi), .onGUIEnd/un.onGUIEnd → SkinEnd (below), every page's SHOW
; (RomPageShow calls it before nsDialogs::Show) → SkinPage. All of them keep the registers they use.

!define SKIN_BG_ID 1999          ; the composed background picture (a static control at the bottom of the z-order)
!define SKIN_CREAM "FFF3D9"
!define SKIN_TEXT "1F1A3A"
!define SKIN_TITLE "B83A06"
!define SKIN_BAR "26305A"
!define SKIN_BAR_TEXT "C9D2F0"
!define SKIN_CREAM_REF 0x00D9F3FF     ; ${SKIN_CREAM} as a COLORREF (0x00BBGGRR)
!define SKIN_FIELD_LINE "1F1A3A"      ; the design's path box border (.b .path: 2px solid #1F1A3A)
!define SKIN_FIELD_LINE_REF 0x003A1A1F
!define SKIN_NEED_PROP 0x5A11    ; window property (an integer atom: nothing to free) with the height a text needs, for the
                                 ; GUI tests (tests/guidrive.nsi fits): Wine cannot read the installer's fonts from outside
!define SKIN_LABEL_ID 1997       ; the "▶ 安裝資料夾" label that takes the directory page's group box's place
!define SKIN_FRAME_SLICE 18      ; make_ui.py FRAME_SLICE (px at 1x)
!define SKIN_PORTRAIT_SLICE 9    ; make_ui.py PORTRAIT_SLICE
!define SKIN_BLEND 0x01FF0000    ; BLENDFUNCTION { AC_SRC_OVER, 0, 255, AC_SRC_ALPHA }
; the layout, px at 1x (times $SkS): the panels' pictures this far from the window's edges, the page this far inside
; the panel's picture (the frame's visible edge is 6 inside it), the header panel's height, the gap between panels
!define SKIN_M 12
!define SKIN_PAD 26
!define SKIN_PADV 20
!define SKIN_HDR_H 74
!define SKIN_HDR_PAD 17      ; room inside the header frame above and below the header texts
!define SKIN_GAP 8

Var SkS          ; 1 or 2: which set of pictures (2 at 144 DPI and up)
Var SkFont       ; path of the title font file in $PLUGINSDIR
Var SkTitleFont  ; welcome/finish title
Var SkHeadFont   ; page header title
Var SkSky
Var SkFrame
Var SkPortrait
Var SkCursor
Var SkCursorS    ; the small ▶ (before a label, before the first check box)
Var SkBarLine
Var SkBarFill
Var SkWizard
Var SkHeader
Var SkDC         ; memory DC being drawn
Var SkSrc        ; DC the pieces are selected into
Var SkDib
Var SkOld
Var SkW          ; size of the target's client area
Var SkH
Var SkOX         ; the target's place in the main window's client area
Var SkOY
Var SkOH         ; height of the main window's client area
Var SkX          ; a rectangle (SkinRect's result; the drawing helpers' target)
Var SkY
Var SkRW
Var SkRH
Var SkBW         ; SkinBitmapSize's result
Var SkBH
Var SkPic        ; picture for the drawing helpers
Var SkPicS       ; its slice at 1x
Var SkM          ; ${SKIN_M} in px
Var SkHdrH       ; the header panel's height in px
Var SkHdrX       ; left and width of the header texts
Var SkHdrW
Var SkHpW        ; the header portrait's width in px
Var SkBarY       ; top of the button bar
Var SkPage       ; the page being shown (SkinPage)
; a page's background picture stays with its static until the page is destroyed (a static does not free its picture):
; kept here and freed once that window is gone (SkinFreePages). Two, because a custom page's predecessor still exists
; while the next one is dressed
Var SkPendBg1
Var SkPendDib1
Var SkPendBg2
Var SkPendDib2

; helpers expanded inside SKIN_FUNCS (they use its ${UN})
!macro SKIN_EXTRACT SET
  File "/oname=$PLUGINSDIR\ui\sky.bmp" "ui\${SET}\sky.bmp"
  File "/oname=$PLUGINSDIR\ui\frame.bmp" "ui\${SET}\frame.bmp"
  File "/oname=$PLUGINSDIR\ui\portrait.bmp" "ui\${SET}\portrait.bmp"
  File "/oname=$PLUGINSDIR\ui\cursor.bmp" "ui\${SET}\cursor.bmp"
  File "/oname=$PLUGINSDIR\ui\cursor_s.bmp" "ui\${SET}\cursor_s.bmp"
  File "/oname=$PLUGINSDIR\ui\barline.bmp" "ui\${SET}\barline.bmp"
  File "/oname=$PLUGINSDIR\ui\barfill.bmp" "ui\${SET}\barfill.bmp"
  ; the uninstaller has no welcome / finish page: the big portrait art only in the installer
  !if "${UN}" == ""
    File "/oname=$PLUGINSDIR\ui\wizard.bmp" "ui\${SET}\wizard.bmp"
  !endif
  File "/oname=$PLUGINSDIR\ui\header.bmp" "ui\${SET}\header.bmp"
!macroend

!macro SKIN_LOAD VAR NAME
  Push "$PLUGINSDIR\ui\${NAME}.bmp"
  Call ${UN}SkinLoad
  Pop ${VAR}
!macroend

; Push frame or portrait; Push slice; Push x; Push y; Push w; Push h; Call SkinBox (nine slices, coordinates of the picture)
!macro SKIN_BOX PIC SLICE X Y W H
  StrCpy $SkPic ${PIC}
  StrCpy $SkPicS ${SLICE}
  StrCpy $SkX ${X}
  StrCpy $SkY ${Y}
  StrCpy $SkRW ${W}
  StrCpy $SkRH ${H}
  Call ${UN}SkinNine
!macroend

; in SkinPlaceMain: button ID moved right by $1 and to the top $9 (uses $0)
!macro SKIN_SHIFT_BUTTON ID
  Push $HWNDPARENT
  Push ${ID}
  Call ${UN}SkinRect
  IntOp $0 $SkX + $1
  GetDlgItem $SkPic $HWNDPARENT ${ID}
  System::Call 'user32::SetWindowPos(p $SkPic, p 0, i r0, i r9, i 0, i 0, i 0x15) i'
!macroend

; in SkinWelcome: control ID of page $0, if there, moved to the left $2, the top $4, the width $3 (MEASURE 0: its
; height kept; 1, a text: what the text takes; 2, a check box or radio button: what its text takes beside the box, at
; least its own height); $4 moves below it by GAP s. Uses $1, $5.
!macro SKIN_STACK ID MEASURE GAP
  GetDlgItem $1 $0 ${ID}
  !insertmacro SKIN_STACK_HWND ${MEASURE} ${GAP}
!macroend

; SKIN_STACK for the control $1 (0 = none), when its ID is not fixed: nsDialogs numbers a page's controls in the order
; they are made (1200, 1201, …), so on the finish page the link's ID depends on how many check boxes come before it
; (SkinFindButton finds it by its kind instead)
!macro SKIN_STACK_HWND MEASURE GAP
  ${If} $1 P<> 0
    !if ${MEASURE} == 1
      ; wrap like an edit control: a run too long for a line is broken, not cut off
      System::Call 'user32::GetWindowLongW(p r1, i -16) i .r5'
      IntOp $5 $5 | 0x2000
      System::Call 'user32::SetWindowLongW(p r1, i -16, i r5) i'
      Push $1
      Push $3
      Call ${UN}SkinTextH
      System::Call 'user32::SetPropW(p r1, p ${SKIN_NEED_PROP}, p $SkRH) i'
    !else if ${MEASURE} == 2
      ; BS_MULTILINE: a text too long for a line wraps. It is measured at the width left of the box and the gap after
      ; it (the box is drawn about SM_CXMENUCHECK wide, the gap half a character: half the box is kept for the gap, on
      ; the safe side) and the control made as tall as that, but not lower than it was (the box itself has to fit)
      System::Call 'user32::GetWindowLongW(p r1, i -16) i .r5'
      IntOp $5 $5 | 0x2000
      System::Call 'user32::SetWindowLongW(p r1, i -16, i r5) i'
      System::Call 'user32::GetSystemMetrics(i 71) i .r5'
      IntOp $5 $5 * 3
      IntOp $5 $5 / 2
      IntOp $5 $3 - $5
      Push $1
      Push $5
      Call ${UN}SkinTextH
      System::Call 'user32::SetPropW(p r1, p ${SKIN_NEED_PROP}, p $SkRH) i'
      StrCpy $5 $SkRH
      Push $1
      Push 0
      Call ${UN}SkinRect
      ${If} $5 > $SkRH
        StrCpy $SkRH $5
      ${EndIf}
    !else
      Push $1
      Push 0
      Call ${UN}SkinRect
    !endif
    System::Call 'user32::SetWindowPos(p r1, p 0, i r2, i r4, i r3, i $SkRH, i 0x14) i'
    IntOp $4 $4 + $SkRH
    IntOp $5 ${GAP} * $SkS
    IntOp $4 $4 + $5
  ${EndIf}
!macroend

; in SkinPage: the main window's text ID measured at its width, the height it needs kept in ${SKIN_NEED_PROP} (uses $0)
!macro SKIN_MEASURE ID
  Push $HWNDPARENT
  Push ${ID}
  Call ${UN}SkinRect
  GetDlgItem $0 $HWNDPARENT ${ID}
  Push $0
  Push $SkRW
  Call ${UN}SkinTextH
  System::Call 'user32::SetPropW(p r0, p ${SKIN_NEED_PROP}, p $SkRH) i'
!macroend

!macro SKIN_HIDE ID
  GetDlgItem $0 $HWNDPARENT ${ID}
  ShowWindow $0 ${SW_HIDE}
!macroend

!macro SKIN_FUNCS UN UNTEXT

; Push hwnd; Push control id (0 = the hwnd itself); Call SkinRect → $SkX $SkY $SkRW $SkRH in the main window's client area
Function ${UN}SkinRect
  Exch $1
  Exch
  Exch $0
  Push $2
  ${If} $1 != 0
    System::Call 'user32::GetDlgItem(p r0, i r1) p .r0'
  ${EndIf}
  System::Call '*(i,i,i,i) p .r2'
  System::Call 'user32::GetWindowRect(p r0, p r2) i'
  System::Call 'user32::MapWindowPoints(p 0, p $HWNDPARENT, p r2, i 2) i'
  System::Call '*$2(i .r0, i .r1)'
  StrCpy $SkX $0
  StrCpy $SkY $1
  System::Call '*$2(i, i, i .r0, i .r1)'
  IntOp $SkRW $0 - $SkX
  IntOp $SkRH $1 - $SkY
  System::Free $2
  Pop $2
  Pop $0
  Pop $1
FunctionEnd

; Push bitmap; Call SkinBitmapSize → $SkBW $SkBH
Function ${UN}SkinBitmapSize
  Exch $0
  Push $1
  Push $2
  System::Call '*(i,i,i,i,i,i,p) p .r1'
  System::Call 'gdi32::GetObjectW(p r0, i 32, p r1) i'
  System::Call '*$1(i, i .r0, i .r2)'
  StrCpy $SkBW $0
  StrCpy $SkBH $2
  System::Free $1
  Pop $2
  Pop $1
  Pop $0
FunctionEnd

; Push file; Call SkinLoad; Pop bitmap (a DIB section, alpha kept)
Function ${UN}SkinLoad
  Exch $0
  System::Call 'user32::LoadImageW(p 0, w r0, i 0, i 0, i 0, i 0x2010) p .r0'
  Exch $0
FunctionEnd

Function ${UN}SkinInit
  Push $0
  Push $1
  InitPluginsDir
  System::Call 'user32::GetDC(p 0) p .r0'
  System::Call 'gdi32::GetDeviceCaps(p r0, i 90) i .r1'
  System::Call 'user32::ReleaseDC(p 0, p r0) i'
  ${If} $1 >= 144
    StrCpy $SkS 2
  ${Else}
    StrCpy $SkS 1
  ${EndIf}
  CreateDirectory "$PLUGINSDIR\ui"
  ${If} $SkS == 2
    !insertmacro SKIN_EXTRACT 2x
  ${Else}
    !insertmacro SKIN_EXTRACT 1x
  ${EndIf}
  ; the title font (OFL 1.1: its licence travels with it), for this program only; removed in SkinEnd
  File "/oname=$PLUGINSDIR\ui\LXGWWenKaiTC-installer.ttf" "ui\LXGWWenKaiTC-installer.ttf"
  File "/oname=$PLUGINSDIR\ui\OFL.txt" "ui\OFL.txt"
  StrCpy $SkFont "$PLUGINSDIR\ui\LXGWWenKaiTC-installer.ttf"
  System::Call 'gdi32::AddFontResourceExW(w "$SkFont", i 0x10, p 0) i'
  CreateFont $SkTitleFont "LXGW WenKai TC" 16 400
  CreateFont $SkHeadFont "LXGW WenKai TC" 14 400
  !insertmacro SKIN_LOAD $SkSky sky
  !insertmacro SKIN_LOAD $SkFrame frame
  !insertmacro SKIN_LOAD $SkPortrait portrait
  !insertmacro SKIN_LOAD $SkCursor cursor
  !insertmacro SKIN_LOAD $SkCursorS cursor_s
  !insertmacro SKIN_LOAD $SkBarLine barline
  !insertmacro SKIN_LOAD $SkBarFill barfill
  !if "${UN}" == ""
    !insertmacro SKIN_LOAD $SkWizard wizard
  !endif
  !insertmacro SKIN_LOAD $SkHeader header
  ; the main window's own texts: header texts on the header panel, the brand text on the bar
  GetDlgItem $0 $HWNDPARENT 1037
  SendMessage $0 ${WM_SETFONT} $SkHeadFont 1
  SetCtlColors $0 ${SKIN_TITLE} ${SKIN_CREAM}
  GetDlgItem $0 $HWNDPARENT 1038
  SetCtlColors $0 ${SKIN_TEXT} ${SKIN_CREAM}
  GetDlgItem $0 $HWNDPARENT 1256
  SetCtlColors $0 ${SKIN_BAR_TEXT} ${SKIN_BAR}
  Call ${UN}SkinPlaceMain
  Pop $1
  Pop $0
FunctionEnd

Function ${UN}SkinEnd
  ${If} $SkS != ""
    ; the font objects made from the title font first, then the font itself
    System::Call 'gdi32::DeleteObject(p $SkTitleFont) i'
    System::Call 'gdi32::DeleteObject(p $SkHeadFont) i'
    System::Call 'gdi32::RemoveFontResourceExW(w "$SkFont", i 0x10, p 0) i'
    System::Call 'gdi32::DeleteObject(p $SkSky) i'
    System::Call 'gdi32::DeleteObject(p $SkFrame) i'
    System::Call 'gdi32::DeleteObject(p $SkPortrait) i'
    System::Call 'gdi32::DeleteObject(p $SkCursor) i'
    System::Call 'gdi32::DeleteObject(p $SkCursorS) i'
    System::Call 'gdi32::DeleteObject(p $SkBarLine) i'
    System::Call 'gdi32::DeleteObject(p $SkBarFill) i'
    System::Call 'gdi32::DeleteObject(p $SkWizard) i'
    System::Call 'gdi32::DeleteObject(p $SkHeader) i'
    Call ${UN}SkinFreePages
  ${EndIf}
FunctionEnd

; Push hwnd; Push width; Call SkinTextH → $SkRH: the height that window's text takes at that width, in its own font
Function ${UN}SkinTextH
  Exch $1
  Exch
  Exch $0
  Push $2
  Push $3
  Push $4
  Push $5
  SendMessage $0 ${WM_GETFONT} 0 0 $2
  System::Call 'user32::GetDC(p r0) p .r3'
  System::Call 'gdi32::SelectObject(p r3, p r2) p .r4'
  System::Call 'user32::GetWindowTextW(p r0, w .r5, i ${NSIS_MAX_STRLEN}) i'
  System::Call '*(i 0, i 0, i r1, i 0) p .r2'
  ; DT_CALCRECT|DT_WORDBREAK|DT_EDITCONTROL, as a static control breaks its lines
  System::Call 'user32::DrawTextW(p r3, w r5, i -1, p r2, i 0x2410) i'
  System::Call '*$2(i, i, i, i .r5)'
  StrCpy $SkRH $5
  System::Free $2
  System::Call 'gdi32::SelectObject(p r3, p r4) p'
  System::Call 'user32::ReleaseDC(p r0, p r3) i'
  Pop $5
  Pop $4
  Pop $3
  Pop $2
  Pop $0
  Pop $1
FunctionEnd

; the header texts for the text they have now: the title one line of its font, the subtext as tall as its text needs
; but at most two lines, the two together centred in the header panel; what each needs is kept for the tests
; (${SKIN_NEED_PROP}: a subtext needing more than two lines shows up there). Called at GUI init, on every page and when
; the progress page puts up its finish / abort header (SkinInstDone)
Function ${UN}SkinHeader
  Push $0
  Push $1
  Push $2
  Push $3
  Push $4
  GetDlgItem $0 $HWNDPARENT 1037
  Push $0
  Push $SkHdrW
  Call ${UN}SkinTextH
  System::Call 'user32::SetPropW(p r0, p ${SKIN_NEED_PROP}, p $SkRH) i'
  Push $0
  Call ${UN}SkinLineH
  StrCpy $2 $SkRH                    ; title height
  GetDlgItem $1 $HWNDPARENT 1038
  System::Call 'user32::GetWindowLongW(p r1, i -16) i .r3'
  IntOp $3 $3 | 0x2000               ; SS_EDITCONTROL: a long run breaks instead of being cut off
  System::Call 'user32::SetWindowLongW(p r1, i -16, i r3) i'
  Push $1
  Push $SkHdrW
  Call ${UN}SkinTextH
  System::Call 'user32::SetPropW(p r1, p ${SKIN_NEED_PROP}, p $SkRH) i'
  StrCpy $3 $SkRH                    ; subtext: what it needs …
  Push $1
  Call ${UN}SkinLineH
  IntOp $4 $SkRH * 2
  ${If} $3 > $4
    StrCpy $3 $4                     ; … at most two lines
  ${EndIf}
  ${If} $3 < $SkRH
    StrCpy $3 $SkRH                  ; … at least one
  ${EndIf}
  IntOp $4 $2 + $3
  IntOp $4 $4 + $SkS
  IntOp $4 $4 + $SkS
  IntOp $4 $SkHdrH - $4
  IntOp $4 $4 / 2
  IntOp $4 $4 + $SkM                 ; title top
  System::Call 'user32::SetWindowPos(p r0, p 0, i $SkHdrX, i r4, i $SkHdrW, i r2, i 0x14) i'
  IntOp $4 $4 + $2
  IntOp $4 $4 + $SkS
  IntOp $4 $4 + $SkS
  System::Call 'user32::SetWindowPos(p r1, p 0, i $SkHdrX, i r4, i $SkHdrW, i r3, i 0x14) i'
  Pop $4
  Pop $3
  Pop $2
  Pop $1
  Pop $0
FunctionEnd

; the progress page's leave (dbz-zhtw.nsi): Modern UI puts up the finish or abort header right after this function
; (Pages/InstallFiles.nsh), with no hook after it; put the same texts up now so SkinHeader lays out and measures them
Function ${UN}SkinInstDone
  ${If} $SkS != ""
    Push $0
    GetDlgItem $0 $HWNDPARENT 1037
    IfAbort skin_abort
      SendMessage $0 ${WM_SETTEXT} 0 "STR:$(MUI_${UNTEXT}TEXT_FINISH_TITLE)"
      GetDlgItem $0 $HWNDPARENT 1038
      SendMessage $0 ${WM_SETTEXT} 0 "STR:$(MUI_${UNTEXT}TEXT_FINISH_SUBTITLE)"
      Goto skin_done
    skin_abort:
      SendMessage $0 ${WM_SETTEXT} 0 "STR:$(MUI_${UNTEXT}TEXT_ABORT_TITLE)"
      GetDlgItem $0 $HWNDPARENT 1038
      SendMessage $0 ${WM_SETTEXT} 0 "STR:$(MUI_${UNTEXT}TEXT_ABORT_SUBTITLE)"
    skin_done:
    Call ${UN}SkinHeader
    Pop $0
  ${EndIf}
FunctionEnd

; the layout, once at GUI init. The pages keep their size (it follows the system font and DPI); the window grows so the
; panels fit around them with the design's room inside: the header panel on top, the page's panel under it, the button
; bar at the bottom. MUI's page placeholders (1018 inner pages, 1044 welcome/finish), the buttons, the header texts
; (in the header panel, left of the header portrait) and the brand text (on the bar) are moved into it.
Function ${UN}SkinPlaceMain
  Push $0
  Push $1
  Push $2
  Push $3
  Push $4
  Push $5
  Push $6
  Push $7
  Push $8
  Push $9
  IntOp $SkM ${SKIN_M} * $SkS
  ; the header panel: ${SKIN_HDR_H}s, or taller when the title (1037, title font) and two lines of the subtext (1038)
  ; with ${SKIN_HDR_PAD}s inside the frame on both sides need more
  IntOp $SkHdrH ${SKIN_HDR_H} * $SkS
  GetDlgItem $1 $HWNDPARENT 1037
  Push $1
  Call ${UN}SkinLineH
  StrCpy $0 $SkRH
  GetDlgItem $1 $HWNDPARENT 1038
  Push $1
  Call ${UN}SkinLineH
  IntOp $0 $0 + $SkRH
  IntOp $0 $0 + $SkRH
  IntOp $0 $0 + $SkS
  IntOp $0 $0 + $SkS
  IntOp $1 ${SKIN_HDR_PAD} * $SkS
  IntOp $0 $0 + $1
  IntOp $0 $0 + $1
  ${If} $0 > $SkHdrH
    StrCpy $SkHdrH $0
  ${EndIf}
  ; the header portrait: as tall as the header panel, the art's shape inside the rings
  Push $SkHeader
  Call ${UN}SkinBitmapSize
  IntOp $0 ${SKIN_PORTRAIT_SLICE} * $SkS
  IntOp $0 $0 * 2
  IntOp $1 $SkHdrH - $0
  IntOp $1 $1 * $SkBW
  IntOp $1 $1 / $SkBH
  IntOp $SkHpW $1 + $0
  ; the inner page: $7 $8 its new place, $5 $6 its size
  Push $HWNDPARENT
  Push 1018
  Call ${UN}SkinRect
  StrCpy $5 $SkRW
  StrCpy $6 $SkRH
  IntOp $0 ${SKIN_PAD} * $SkS
  IntOp $7 $SkM + $0
  IntOp $1 ${SKIN_GAP} * $SkS
  IntOp $2 ${SKIN_PADV} * $SkS
  IntOp $8 $SkM + $SkHdrH
  IntOp $8 $8 + $1
  IntOp $8 $8 + $2
  ; the bar: under the page's panel and a gap
  IntOp $SkBarY $8 + $6
  IntOp $SkBarY $SkBarY + $2
  IntOp $SkBarY $SkBarY + $1
  ; the new client area $3 x $4: buttons ($9 their top) 12s under the bar's 3s line, 12s above the bottom
  IntOp $3 $7 + $5
  IntOp $3 $3 + $7
  Push $HWNDPARENT
  Push 1
  Call ${UN}SkinRect
  IntOp $9 15 * $SkS
  IntOp $9 $SkBarY + $9
  IntOp $4 12 * $SkS
  IntOp $4 $4 + $9
  IntOp $4 $4 + $SkRH
  ; grow the window by the difference ($1 $2), keeping it centred
  System::Call '*(i,i,i,i) p .r0'
  System::Call 'user32::GetClientRect(p $HWNDPARENT, p r0) i'
  System::Call '*$0(i, i, i .r1, i .r2)'
  IntOp $1 $3 - $1
  IntOp $2 $4 - $2
  System::Call 'user32::GetWindowRect(p $HWNDPARENT, p r0) i'
  System::Call '*$0(i .r5, i .r6)'
  System::Call '*$0(i, i, i .r4)'
  IntOp $4 $4 - $5
  IntOp $4 $4 + $1                   ; new outer width
  Push $4
  System::Call '*$0(i, i, i, i .r4)'
  IntOp $4 $4 - $6
  IntOp $4 $4 + $2                   ; new outer height
  System::Free $0
  IntOp $0 $1 / 2
  IntOp $5 $5 - $0
  IntOp $0 $2 / 2
  IntOp $6 $6 - $0
  Pop $0
  System::Call 'user32::SetWindowPos(p $HWNDPARENT, p 0, i r5, i r6, i r0, i r4, i 0x14) i'
  ; the page placeholders
  GetDlgItem $0 $HWNDPARENT 1018
  System::Call 'user32::SetWindowPos(p r0, p 0, i r7, i r8, i 0, i 0, i 0x15) i'
  GetDlgItem $0 $HWNDPARENT 1044
  System::Call 'user32::SetWindowPos(p r0, p 0, i 0, i 0, i r3, i $SkBarY, i 0x14) i'
  ; the buttons: as far from the right edge as before, on the bar
  !insertmacro SKIN_SHIFT_BUTTON 3
  !insertmacro SKIN_SHIFT_BUTTON 1
  !insertmacro SKIN_SHIFT_BUTTON 2
  ; Modern UI puts Back right against Next (dialog units rounded to pixels can overlap them by one): no overlap
  Push $HWNDPARENT
  Push 1
  Call ${UN}SkinRect
  StrCpy $0 $SkX
  Push $HWNDPARENT
  Push 3
  Call ${UN}SkinRect
  IntOp $1 $SkX + $SkRW
  ${If} $1 > $0
    IntOp $1 $1 - $0
    IntOp $1 $SkX - $1
    GetDlgItem $2 $HWNDPARENT 3
    System::Call 'user32::SetWindowPos(p r2, p 0, i r1, i r9, i 0, i 0, i 0x15) i'
  ${EndIf}
  ; header texts: from the page's left edge to the header portrait less 8s (their heights and places: SkinHeader)
  StrCpy $SkHdrX $7
  IntOp $5 $3 - $SkM
  IntOp $5 $5 - $SkHpW
  IntOp $0 8 * $SkS
  IntOp $5 $5 - $0
  IntOp $SkHdrW $5 - $7
  Call ${UN}SkinHeader
  ; brand text: on the bar from the frames' visible edge to 12s before the first button, centred on the buttons
  Push $HWNDPARENT
  Push 3
  Call ${UN}SkinRect
  StrCpy $5 $SkX
  IntOp $6 $SkRH / 2
  IntOp $6 $SkY + $6                 ; buttons' middle
  Push $HWNDPARENT
  Push 1256
  Call ${UN}SkinRect
  IntOp $0 $SkRH / 2
  IntOp $6 $6 - $0                   ; top
  IntOp $7 6 * $SkS
  IntOp $7 $7 + $SkM                 ; left
  IntOp $0 12 * $SkS
  IntOp $5 $5 - $0
  IntOp $5 $5 - $7                   ; width
  GetDlgItem $1 $HWNDPARENT 1256
  System::Call 'user32::SetWindowPos(p r1, p 0, i r7, i r6, i r5, i $SkRH, i 0x14) i'
  Pop $9
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

; Push hwnd; Call SkinBegin: a memory picture the size of that window's client area with the sky drawn in (the part of
; a gradient as tall as the main window's client area, so it continues across windows)
Function ${UN}SkinBegin
  Exch $0
  Push $1
  Push $2
  Push $3
  System::Call '*(i,i,i,i) p .r1'
  System::Call 'user32::GetClientRect(p $HWNDPARENT, p r1) i'
  System::Call '*$1(i, i, i, i .r2)'
  StrCpy $SkOH $2
  System::Call 'user32::GetClientRect(p r0, p r1) i'
  System::Call '*$1(i, i, i .r2, i .r3)'
  StrCpy $SkW $2
  StrCpy $SkH $3
  System::Call '*$1(i 0, i 0, i 0, i 0)'
  System::Call 'user32::MapWindowPoints(p r0, p $HWNDPARENT, p r1, i 1) i'
  System::Call '*$1(i .r2, i .r3)'
  StrCpy $SkOX $2
  StrCpy $SkOY $3
  System::Free $1
  System::Call 'user32::GetDC(p 0) p .r1'
  System::Call 'gdi32::CreateCompatibleDC(p r1) p .r2'
  StrCpy $SkDC $2
  System::Call 'gdi32::CreateCompatibleDC(p r1) p .r2'
  StrCpy $SkSrc $2
  ; a 24-bit DIB: the static shows it as it is (no alpha)
  System::Call '*(i 40, i $SkW, i $SkH, &i2 1, &i2 24, i 0, i 0, i 0, i 0, i 0, i 0) p .r2'
  System::Call 'gdi32::CreateDIBSection(p r1, p r2, i 0, *p .r3, p 0, i 0) p .r3'
  StrCpy $SkDib $3
  System::Free $2
  System::Call 'user32::ReleaseDC(p 0, p r1) i'
  System::Call 'gdi32::SelectObject(p $SkDC, p $SkDib) p .r2'
  StrCpy $SkOld $2
  System::Call 'gdi32::SetStretchBltMode(p $SkDC, i 4) i'
  System::Call 'gdi32::SetBrushOrgEx(p $SkDC, i 0, i 0, p 0) i'
  Push $SkSky
  Call ${UN}SkinBitmapSize
  IntOp $2 $SkOY * $SkBH
  IntOp $2 $2 / $SkOH                ; source top
  IntOp $3 $SkH * $SkBH
  IntOp $3 $3 / $SkOH                ; source height
  ${If} $3 < 1
    StrCpy $3 1
  ${EndIf}
  System::Call 'gdi32::SelectObject(p $SkSrc, p $SkSky) p'
  System::Call 'gdi32::StretchBlt(p $SkDC, i 0, i 0, i $SkW, i $SkH, p $SkSrc, i 0, i r2, i $SkBW, i r3, i 0xCC0020) i'
  Pop $3
  Pop $2
  Pop $1
  Pop $0
FunctionEnd

; $SkPic (32-bit, premultiplied) with slice $SkPicS (at 1x) → nine slices into $SkX $SkY $SkRW $SkRH (AlphaBlend)
Function ${UN}SkinNine
  Push $0
  Push $1
  Push $2
  Push $3
  Push $4
  Push $5
  Push $6
  Push $7
  Push $8
  Push $9
  Push $SkPic
  Call ${UN}SkinBitmapSize
  IntOp $0 $SkPicS * $SkS            ; slice s
  IntOp $1 $SkBW - $0
  IntOp $1 $1 - $0                   ; picture middle width
  IntOp $2 $SkBH - $0
  IntOp $2 $2 - $0                   ; picture middle height
  IntOp $3 $SkRW - $0
  IntOp $3 $3 - $0                   ; target middle width
  IntOp $4 $SkRH - $0
  IntOp $4 $4 - $0                   ; target middle height
  IntOp $5 $SkX + $0                 ; x1
  IntOp $6 $SkX + $SkRW
  IntOp $6 $6 - $0                   ; x2
  IntOp $7 $SkY + $0                 ; y1
  IntOp $8 $SkY + $SkRH
  IntOp $8 $8 - $0                   ; y2
  IntOp $9 $0 + $1                   ; source x2
  System::Call 'gdi32::SelectObject(p $SkSrc, p $SkPic) p'
  System::Call 'msimg32::AlphaBlend(p $SkDC, i $SkX, i $SkY, i r0, i r0, p $SkSrc, i 0, i 0, i r0, i r0, i ${SKIN_BLEND}) i'
  System::Call 'msimg32::AlphaBlend(p $SkDC, i r5, i $SkY, i r3, i r0, p $SkSrc, i r0, i 0, i r1, i r0, i ${SKIN_BLEND}) i'
  System::Call 'msimg32::AlphaBlend(p $SkDC, i r6, i $SkY, i r0, i r0, p $SkSrc, i r9, i 0, i r0, i r0, i ${SKIN_BLEND}) i'
  System::Call 'msimg32::AlphaBlend(p $SkDC, i $SkX, i r7, i r0, i r4, p $SkSrc, i 0, i r0, i r0, i r2, i ${SKIN_BLEND}) i'
  System::Call 'msimg32::AlphaBlend(p $SkDC, i r5, i r7, i r3, i r4, p $SkSrc, i r0, i r0, i r1, i r2, i ${SKIN_BLEND}) i'
  System::Call 'msimg32::AlphaBlend(p $SkDC, i r6, i r7, i r0, i r4, p $SkSrc, i r9, i r0, i r0, i r2, i ${SKIN_BLEND}) i'
  IntOp $9 $0 + $2                   ; source y2
  System::Call 'msimg32::AlphaBlend(p $SkDC, i $SkX, i r8, i r0, i r0, p $SkSrc, i 0, i r9, i r0, i r0, i ${SKIN_BLEND}) i'
  System::Call 'msimg32::AlphaBlend(p $SkDC, i r5, i r8, i r3, i r0, p $SkSrc, i r0, i r9, i r1, i r0, i ${SKIN_BLEND}) i'
  IntOp $1 $0 + $1                   ; source x2 again
  System::Call 'msimg32::AlphaBlend(p $SkDC, i r6, i r8, i r0, i r0, p $SkSrc, i r1, i r9, i r0, i r0, i ${SKIN_BLEND}) i'
  Pop $9
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

; $SkPic (24-bit art) → $SkX $SkY $SkRW $SkRH, filling it (scaled to cover, centred, the rest cropped)
Function ${UN}SkinArt
  Push $0
  Push $1
  Push $2
  Push $3
  Push $SkPic
  Call ${UN}SkinBitmapSize
  ; picture wider than the target (w/h): crop its sides, else crop top and bottom
  IntOp $0 $SkBW * $SkRH
  IntOp $1 $SkRW * $SkBH
  ${If} $0 > $1
    IntOp $2 $SkBH * $SkRW
    IntOp $2 $2 / $SkRH              ; source width
    IntOp $0 $SkBW - $2
    IntOp $0 $0 / 2                  ; source x
    StrCpy $1 0
    StrCpy $3 $SkBH
  ${Else}
    StrCpy $2 $SkBW
    IntOp $3 $SkBW * $SkRH
    IntOp $3 $3 / $SkRW              ; source height
    StrCpy $0 0
    IntOp $1 $SkBH - $3
    IntOp $1 $1 / 2
  ${EndIf}
  System::Call 'gdi32::SelectObject(p $SkSrc, p $SkPic) p'
  System::Call 'gdi32::StretchBlt(p $SkDC, i $SkX, i $SkY, i $SkRW, i $SkRH, p $SkSrc, i r0, i r1, i r2, i r3, i 0xCC0020) i'
  Pop $3
  Pop $2
  Pop $1
  Pop $0
FunctionEnd

; the ▶ cursor at $SkX $SkY
Function ${UN}SkinCursor
  Push $SkCursor
  Call ${UN}SkinBitmapSize
  System::Call 'gdi32::SelectObject(p $SkSrc, p $SkCursor) p'
  System::Call 'msimg32::AlphaBlend(p $SkDC, i $SkX, i $SkY, i $SkBW, i $SkBH, p $SkSrc, i 0, i 0, i $SkBW, i $SkBH, i ${SKIN_BLEND}) i'
FunctionEnd

; the small ▶ at $SkX $SkY
Function ${UN}SkinCursorS
  Push $SkCursorS
  Call ${UN}SkinBitmapSize
  System::Call 'gdi32::SelectObject(p $SkSrc, p $SkCursorS) p'
  System::Call 'msimg32::AlphaBlend(p $SkDC, i $SkX, i $SkY, i $SkBW, i $SkBH, p $SkSrc, i 0, i 0, i $SkBW, i $SkBH, i ${SKIN_BLEND}) i'
FunctionEnd

; free the kept page pictures whose static is gone
Function ${UN}SkinFreePages
  Push $0
  ${If} $SkPendDib1 != ""
    System::Call 'user32::IsWindow(p $SkPendBg1) i .r0'
    ${If} $0 = 0
      System::Call 'gdi32::DeleteObject(p $SkPendDib1) i'
      StrCpy $SkPendDib1 ""
    ${EndIf}
  ${EndIf}
  ${If} $SkPendDib2 != ""
    System::Call 'user32::IsWindow(p $SkPendBg2) i .r0'
    ${If} $0 = 0
      System::Call 'gdi32::DeleteObject(p $SkPendDib2) i'
      StrCpy $SkPendDib2 ""
    ${EndIf}
  ${EndIf}
  Pop $0
FunctionEnd

; Push hwnd; Call SkinDone: the picture becomes that window's background (static ${SKIN_BG_ID} at the bottom)
Function ${UN}SkinDone
  Exch $0
  Push $1
  Push $2
  System::Call 'gdi32::SelectObject(p $SkDC, p $SkOld) p'
  System::Call 'gdi32::DeleteDC(p $SkDC) i'
  System::Call 'gdi32::DeleteDC(p $SkSrc) i'
  System::Call 'user32::GetDlgItem(p r0, i ${SKIN_BG_ID}) p .r1'
  ${If} $1 P= 0
    ; WS_CHILD|WS_VISIBLE|WS_CLIPSIBLINGS|SS_BITMAP: it never paints over the controls above it
    System::Call 'user32::CreateWindowExW(i 0, w "STATIC", p 0, i 0x5400000E, i 0, i 0, i $SkW, i $SkH, p r0, p ${SKIN_BG_ID}, p 0, p 0) p .r1'
  ${EndIf}
  ; HWND_BOTTOM; SWP_NOMOVE|SWP_NOSIZE|SWP_NOACTIVATE
  System::Call 'user32::SetWindowPos(p r1, p 1, i 0, i 0, i 0, i 0, i 0x13) i'
  SendMessage $1 0x172 0 $SkDib $2    ; STM_SETIMAGE, IMAGE_BITMAP
  ${If} $2 <> 0
    System::Call 'gdi32::DeleteObject(p r2) i'
    ; the same page dressed again: its old picture is freed here, so it must not stay in the list below
    ${If} $SkPendDib1 == $2
      StrCpy $SkPendDib1 ""
    ${EndIf}
    ${If} $SkPendDib2 == $2
      StrCpy $SkPendDib2 ""
    ${EndIf}
  ${EndIf}
  ; a page's picture: freed after the page is gone (the main window's is replaced above, page by page)
  ${If} $0 != $HWNDPARENT
    Call ${UN}SkinFreePages
    ${If} $SkPendDib1 == ""
      StrCpy $SkPendBg1 $1
      StrCpy $SkPendDib1 $SkDib
    ${ElseIf} $SkPendDib2 == ""
      StrCpy $SkPendBg2 $1
      StrCpy $SkPendDib2 $SkDib
    ${EndIf}
  ${EndIf}
  Pop $2
  Pop $1
  Pop $0
FunctionEnd

; the main window: sky; on inner pages the page's panel and the header panel with the header portrait at its right end;
; the button bar from $SkBarY down
Function ${UN}SkinMain
  Push $0
  Push $1
  Push $2
  Push $3
  Push $4
  Push $5
  Push $HWNDPARENT
  Call ${UN}SkinBegin
  Push $SkPage
  Push 0
  Call ${UN}SkinRect
  ${If} $SkX > 0
    ; the page's panel: across the window less the margins, ${SKIN_PADV}s above and below the page
    IntOp $0 ${SKIN_PADV} * $SkS
    IntOp $2 $SkY - $0
    IntOp $4 $SkRH + $0
    IntOp $4 $4 + $0
    IntOp $3 $SkW - $SkM
    IntOp $3 $3 - $SkM
    !insertmacro SKIN_BOX $SkFrame ${SKIN_FRAME_SLICE} $SkM $2 $3 $4
    ; the header panel
    !insertmacro SKIN_BOX $SkFrame ${SKIN_FRAME_SLICE} $SkM $SkM $3 $SkHdrH
    ; the header portrait at its right end, as tall as it
    IntOp $1 $SkW - $SkM
    IntOp $1 $1 - $SkHpW
    !insertmacro SKIN_BOX $SkPortrait ${SKIN_PORTRAIT_SLICE} $1 $SkM $SkHpW $SkHdrH
    IntOp $0 ${SKIN_PORTRAIT_SLICE} * $SkS
    IntOp $SkX $1 + $0
    IntOp $SkY $SkM + $0
    IntOp $SkRW $SkHpW - $0
    IntOp $SkRW $SkRW - $0
    IntOp $SkRH $SkHdrH - $0
    IntOp $SkRH $SkRH - $0
    StrCpy $SkPic $SkHeader
    Call ${UN}SkinArt
    ; the header panel's right studs on top of the portrait, as in the design: in the frame picture the studs are
    ; opaque 14s squares 3s in from its edges (pieces.html #frame i: 6px out from the frame's padding box, inside its
    ; 3px border), copied to the same place of the panel
    Push $SkFrame
    Call ${UN}SkinBitmapSize
    IntOp $0 14 * $SkS                 ; stud size
    IntOp $5 3 * $SkS                  ; its inset
    IntOp $1 $SkBW - $0
    IntOp $1 $1 - $5                   ; in the picture: x of the right studs
    IntOp $2 $SkBH - $0
    IntOp $2 $2 - $5                   ; y of the bottom one
    IntOp $3 $SkW - $SkM
    IntOp $3 $3 - $0
    IntOp $3 $3 - $5                   ; on the window
    IntOp $4 $SkM + $SkHdrH
    IntOp $4 $4 - $0
    IntOp $4 $4 - $5
    System::Call 'gdi32::SelectObject(p $SkSrc, p $SkFrame) p'
    IntOp $SkY $SkM + $5               ; on the window: y of the top one
    System::Call 'msimg32::AlphaBlend(p $SkDC, i r3, i $SkY, i r0, i r0, p $SkSrc, i r1, i r5, i r0, i r0, i ${SKIN_BLEND}) i'
    System::Call 'msimg32::AlphaBlend(p $SkDC, i r3, i r4, i r0, i r0, p $SkSrc, i r1, i r2, i r0, i r0, i ${SKIN_BLEND}) i'
  ${EndIf}
  ; the button bar: the red-orange line, dark blue below
  StrCpy $0 $SkBarY
  Push $SkBarLine
  Call ${UN}SkinBitmapSize
  System::Call 'gdi32::SelectObject(p $SkSrc, p $SkBarLine) p'
  System::Call 'gdi32::StretchBlt(p $SkDC, i 0, i r0, i $SkW, i $SkBH, p $SkSrc, i 0, i 0, i $SkBW, i $SkBH, i 0xCC0020) i'
  IntOp $0 $0 + $SkBH
  IntOp $1 $SkH - $0
  Push $SkBarFill
  Call ${UN}SkinBitmapSize
  System::Call 'gdi32::SelectObject(p $SkSrc, p $SkBarFill) p'
  System::Call 'gdi32::StretchBlt(p $SkDC, i 0, i r0, i $SkW, i r1, p $SkSrc, i 0, i 0, i $SkBW, i $SkBH, i 0xCC0020) i'
  Push $HWNDPARENT
  Call ${UN}SkinDone
  Pop $5
  Pop $4
  Pop $3
  Pop $2
  Pop $1
  Pop $0
FunctionEnd

; the welcome / finish page $0: the portrait with the Goku art (MUI's picture 1200 hidden) as tall as the page less the
; margins, the art's shape; the cream panel right of it; inside the panel, from the top: ▶ and the title (title font),
; the text, then on the finish page the link and the check box (▶ before it, like the game's command menu)
Function ${UN}SkinWelcome
  Push $1
  Push $2
  Push $3
  Push $4
  Push $5
  Push $6
  Push $0
  Call ${UN}SkinBegin
  GetDlgItem $1 $0 1200
  ShowWindow $1 ${SW_HIDE}
  ; MUI hides the brand text on these pages because they used to cover the bar; now they end above it
  GetDlgItem $1 $HWNDPARENT 1256
  ShowWindow $1 ${SW_SHOW}
  ; the portrait: $4 its height, $3 its width
  IntOp $4 $SkH - $SkM
  IntOp $4 $4 - $SkM
  IntOp $2 ${SKIN_PORTRAIT_SLICE} * $SkS
  IntOp $5 $2 * 2
  Push $SkWizard
  Call ${UN}SkinBitmapSize
  IntOp $3 $4 - $5
  IntOp $3 $3 * $SkBW
  IntOp $3 $3 / $SkBH
  IntOp $3 $3 + $5
  !insertmacro SKIN_BOX $SkPortrait ${SKIN_PORTRAIT_SLICE} $SkM $SkM $3 $4
  IntOp $SkX $SkM + $2
  StrCpy $SkY $SkX
  IntOp $SkRW $3 - $5
  IntOp $SkRH $4 - $5
  StrCpy $SkPic $SkWizard
  Call ${UN}SkinArt
  ; the panel: $6 its left, from the portrait + the gap to the right margin
  IntOp $6 $SkM + $3
  IntOp $1 ${SKIN_GAP} * $SkS
  IntOp $6 $6 + $1
  IntOp $3 $SkW - $SkM
  IntOp $3 $3 - $6
  !insertmacro SKIN_BOX $SkFrame ${SKIN_FRAME_SLICE} $6 $SkM $3 $4
  ; inside: $2 left, $3 width, $4 top
  Push $SkCursor
  Call ${UN}SkinBitmapSize
  IntOp $1 ${SKIN_PAD} * $SkS
  IntOp $2 $6 + $1
  IntOp $3 $3 - $1
  IntOp $3 $3 - $1
  IntOp $4 ${SKIN_PADV} * $SkS
  IntOp $4 $4 + $SkM
  ; the title: ▶ at the left, the title after it in the title font
  GetDlgItem $1 $0 1201
  SendMessage $1 ${WM_SETFONT} $SkTitleFont 1
  SetCtlColors $1 ${SKIN_TITLE} ${SKIN_CREAM}
  StrCpy $SkX $2
  IntOp $SkY 4 * $SkS
  IntOp $SkY $SkY + $4
  Call ${UN}SkinCursor
  IntOp $2 $2 + $SkBW
  IntOp $2 $2 + $SkS
  IntOp $3 $3 - $SkBW
  IntOp $3 $3 - $SkS
  !insertmacro SKIN_STACK 1201 1 10
  IntOp $2 $2 - $SkBW
  IntOp $2 $2 - $SkS
  IntOp $3 $3 + $SkBW
  IntOp $3 $3 + $SkS
  !insertmacro SKIN_STACK 1202 1 10
  ; the link (Ko-fi) and the check box are found by their kind, not their ID: nsDialogs numbers the controls in the
  ; order MUI makes them, and the link comes after the check boxes, so its ID moves when there are fewer of them
  ; (the HD pack's box left this page and the link became 1204 — a fixed 1205 left it where MUI had put it)
  Push $0
  Push 0
  Push "link"
  Call ${UN}SkinFindButton
  Pop $1
  !insertmacro SKIN_STACK_HWND 0 14
  ; the check box (立即開啟遊戲): the small ▶ before it (the design's .menu li.on), the box as tall as its text needs
  ; (a long one wraps); $6 (the panel's left, no longer needed) keeps its top for the ▶
  Push $SkCursorS
  Call ${UN}SkinBitmapSize
  IntOp $5 3 * $SkS
  IntOp $5 $5 + $SkBW
  IntOp $2 $2 + $5
  IntOp $3 $3 - $5
  StrCpy $6 $4
  Push $0
  Push 0
  Push "box"
  Call ${UN}SkinFindButton
  Pop $1
  !insertmacro SKIN_STACK_HWND 2 6
  ${If} $1 P<> 0
    Push $1
    Push 0
    Call ${UN}SkinRect
    Push $SkCursorS
    Call ${UN}SkinBitmapSize
    IntOp $5 3 * $SkS
    IntOp $5 $5 + $SkBW
    IntOp $SkX $2 - $5
    IntOp $5 $SkRH - $SkBH
    IntOp $5 $5 / 2
    IntOp $SkY $6 + $5
    Call ${UN}SkinCursorS
  ${EndIf}
  ; any further check box below it, without a ▶
  Push $0
  Push 1
  Push "box"
  Call ${UN}SkinFindButton
  Pop $1
  !insertmacro SKIN_STACK_HWND 2 6
  Push $0
  Call ${UN}SkinDone
  Pop $6
  Pop $5
  Pop $4
  Pop $3
  Pop $2
  Pop $1
FunctionEnd

; Push page; Push n; Push "link" or "box"; Call SkinFindButton; Pop hwnd: the n-th (from 0) child of the page that is
; a link (nsDialogs' Link: an owner-drawn button, BS_OWNERDRAW) or a check box / radio button (BS_CHECKBOX …
; BS_AUTO3STATE, BS_AUTORADIOBUTTON), in the order they were made; 0 when there is none
Function ${UN}SkinFindButton
  Exch $2
  Exch
  Exch $1
  Exch 2
  Exch $0
  Push $3
  Push $4
  Push $5
  StrCpy $5 0
  System::Call 'user32::GetWindow(p r0, i 5) p .r3'
  ${DoWhile} $3 P<> 0
    System::Call 'user32::GetClassNameW(p r3, w .r4, i 64) i'
    ${If} $4 == "Button"
      System::Call 'user32::GetWindowLongW(p r3, i -16) i .r4'
      IntOp $4 $4 & 0xF
      ${If} $2 == "link"
        ${If} $4 = 11
          StrCpy $4 "hit"
        ${EndIf}
      ${ElseIf} $4 = 9
        StrCpy $4 "hit"
      ${ElseIf} $4 >= 2
      ${AndIf} $4 <= 6
        StrCpy $4 "hit"
      ${EndIf}
      ${If} $4 == "hit"
        ${If} $1 = 0
          StrCpy $5 $3
          ${ExitDo}
        ${EndIf}
        IntOp $1 $1 - 1
      ${EndIf}
    ${EndIf}
    System::Call 'user32::GetWindow(p r3, i 2) p .r3'
  ${Loop}
  StrCpy $0 $5
  Pop $5
  Pop $4
  Pop $3
  ; the stack is now: the caller's $0, $2, $1; it is left holding only the result
  Exch $0
  Exch
  Pop $2
  Exch
  Pop $1
FunctionEnd

; Push parent; Push hwnd; Call SkinRectIn → $SkX $SkY $SkRW $SkRH of hwnd in the parent's client area
Function ${UN}SkinRectIn
  Exch $1
  Exch
  Exch $0
  Push $2
  System::Call '*(i,i,i,i) p .r2'
  System::Call 'user32::GetWindowRect(p r1, p r2) i'
  System::Call 'user32::MapWindowPoints(p 0, p r0, p r2, i 2) i'
  System::Call '*$2(i .r0, i .r1)'
  StrCpy $SkX $0
  StrCpy $SkY $1
  System::Call '*$2(i, i, i .r0, i .r1)'
  IntOp $SkRW $0 - $SkX
  IntOp $SkRH $1 - $SkY
  System::Free $2
  Pop $2
  Pop $0
  Pop $1
FunctionEnd

; Push hwnd; Call SkinLineH → $SkRH: the height of one line in that window's font
Function ${UN}SkinLineH
  Exch $0
  Push $1
  Push $2
  Push $3
  Push $4
  SendMessage $0 ${WM_GETFONT} 0 0 $1
  System::Call 'user32::GetDC(p r0) p .r2'
  System::Call 'gdi32::SelectObject(p r2, p r1) p .r1'
  System::Call '*(&i64) p .r3'
  System::Call 'gdi32::GetTextMetricsW(p r2, p r3) i'
  System::Call '*$3(i .r4)'
  StrCpy $SkRH $4
  System::Free $3
  System::Call 'gdi32::SelectObject(p r2, p r1) p'
  System::Call 'user32::ReleaseDC(p r0, p r2) i'
  Pop $4
  Pop $3
  Pop $2
  Pop $1
  Pop $0
FunctionEnd

; Push hwnd; Push y; Push delta; Call SkinShift: on the page $SkPage every other control whose top is at y or below
; moves down by delta
Function ${UN}SkinShift
  Exch $2
  Exch
  Exch $1
  Exch 2
  Exch $0
  Push $3
  Push $4
  System::Call 'user32::GetWindow(p $SkPage, i 5) p .r3'
  ${DoWhile} $3 P<> 0
    ${If} $3 P<> $0
      Push $SkPage
      Push $3
      Call ${UN}SkinRectIn
      ${If} $SkY >= $1
        IntOp $4 $SkY + $2
        System::Call 'user32::SetWindowPos(p r3, p 0, i $SkX, i r4, i 0, i 0, i 0x15) i'
      ${EndIf}
    ${EndIf}
    System::Call 'user32::GetWindow(p r3, i 2) p .r3'
  ${Loop}
  Pop $4
  Pop $3
  Pop $0
  Pop $2
  Pop $1
FunctionEnd

; Push hwnd (a text on the page $SkPage); Call SkinFit: it wraps like an edit control (a run too long for a line is
; broken, not cut off); when its text needs more height than it has, it gets that height and everything below it on
; the page moves down by the difference
Function ${UN}SkinFit
  Exch $0
  Push $1
  Push $2
  Push $3
  Push $4
  System::Call 'user32::GetWindowLongW(p r0, i -16) i .r1'
  IntOp $1 $1 | 0x2000               ; SS_EDITCONTROL
  System::Call 'user32::SetWindowLongW(p r0, i -16, i r1) i'
  Push $SkPage
  Push $0
  Call ${UN}SkinRectIn
  StrCpy $1 $SkX
  StrCpy $2 $SkY
  StrCpy $3 $SkRW
  StrCpy $4 $SkRH
  Push $0
  Push $3
  Call ${UN}SkinTextH
  System::Call 'user32::SetPropW(p r0, p ${SKIN_NEED_PROP}, p $SkRH) i'
  ${If} $SkRH > $4
    System::Call 'user32::SetWindowPos(p r0, p 0, i r1, i r2, i r3, i $SkRH, i 0x14) i'
    IntOp $1 $2 + $4                 ; the old bottom
    IntOp $1 $1 - 1
    IntOp $4 $SkRH - $4
    Push $0
    Push $1
    Push $4
    Call ${UN}SkinShift
  ${EndIf}
  Pop $4
  Pop $3
  Pop $2
  Pop $1
  Pop $0
FunctionEnd

; Push hwnd (a check box or radio button on the page $SkPage); Call SkinFitBox: as SkinFit, with BS_MULTILINE, its text
; measured at its width less the box and the gap (as SKIN_STACK 2 does); never made lower than it is (the box has to fit)
Function ${UN}SkinFitBox
  Exch $0
  Push $1
  Push $2
  Push $3
  Push $4
  Push $5
  System::Call 'user32::GetWindowLongW(p r0, i -16) i .r1'
  IntOp $1 $1 | 0x2000               ; BS_MULTILINE
  System::Call 'user32::SetWindowLongW(p r0, i -16, i r1) i'
  Push $SkPage
  Push $0
  Call ${UN}SkinRectIn
  StrCpy $1 $SkX
  StrCpy $2 $SkY
  StrCpy $3 $SkRW
  StrCpy $4 $SkRH
  System::Call 'user32::GetSystemMetrics(i 71) i .r5'   ; SM_CXMENUCHECK
  IntOp $5 $5 * 3
  IntOp $5 $5 / 2
  IntOp $5 $3 - $5
  Push $0
  Push $5
  Call ${UN}SkinTextH
  System::Call 'user32::SetPropW(p r0, p ${SKIN_NEED_PROP}, p $SkRH) i'
  ${If} $SkRH > $4
    System::Call 'user32::SetWindowPos(p r0, p 0, i r1, i r2, i r3, i $SkRH, i 0x14) i'
    IntOp $1 $2 + $4                 ; the old bottom
    IntOp $1 $1 - 1
    IntOp $4 $SkRH - $4
    Push $0
    Push $1
    Push $4
    Call ${UN}SkinShift
  ${EndIf}
  Pop $5
  Pop $4
  Pop $3
  Pop $2
  Pop $1
  Pop $0
FunctionEnd

; the page $SkPage: if its controls now reach below it, the tree (components page) gives up that much height
Function ${UN}SkinOverflow
  Push $0
  Push $1
  Push $2
  Push $3
  System::Call '*(i,i,i,i) p .r0'
  System::Call 'user32::GetClientRect(p $SkPage, p r0) i'
  System::Call '*$0(i, i, i, i .r3)'
  System::Free $0
  StrCpy $2 0                        ; lowest bottom
  System::Call 'user32::GetWindow(p $SkPage, i 5) p .r0'
  ${DoWhile} $0 P<> 0
    System::Call 'user32::GetWindowLongW(p r0, i -16) i .r1'
    IntOp $1 $1 & 0x10000000         ; WS_VISIBLE
    ${If} $1 <> 0
      System::Call 'user32::GetDlgCtrlID(p r0) i .r1'
      ${If} $1 <> ${SKIN_BG_ID}
        Push $SkPage
        Push $0
        Call ${UN}SkinRectIn
        IntOp $1 $SkY + $SkRH
        ${If} $1 > $2
          StrCpy $2 $1
        ${EndIf}
      ${EndIf}
    ${EndIf}
    System::Call 'user32::GetWindow(p r0, i 2) p .r0'
  ${Loop}
  IntOp $2 $2 - $3                   ; how much too low
  ${If} $2 > 0
    FindWindow $0 "SysTreeView32" "" $SkPage
    ${If} $0 P<> 0
      Push $SkPage
      Push $0
      Call ${UN}SkinRectIn
      IntOp $1 $SkRH - $2
      System::Call 'user32::SetWindowPos(p r0, p 0, i 0, i 0, i $SkRW, i r1, i 0x16) i'
    ${EndIf}
    ; what still reaches below the page (the components page's space required, at the bottom of the left column)
    ; moves up to end at the page's bottom
    System::Call 'user32::GetWindow(p $SkPage, i 5) p .r0'
    ${DoWhile} $0 P<> 0
      System::Call 'user32::GetWindowLongW(p r0, i -16) i .r1'
      IntOp $1 $1 & 0x10000000
      ${If} $1 <> 0
        Push $SkPage
        Push $0
        Call ${UN}SkinRectIn
        IntOp $1 $SkY + $SkRH
        ${If} $1 > $3
          IntOp $1 $3 - $SkRH
          System::Call 'user32::SetWindowPos(p r0, p 0, i $SkX, i r1, i 0, i 0, i 0x15) i'
        ${EndIf}
      ${EndIf}
      System::Call 'user32::GetWindow(p r0, i 2) p .r0'
    ${Loop}
  ${EndIf}
  Pop $3
  Pop $2
  Pop $1
  Pop $0
FunctionEnd

; the directory page (group box 1020 around the field 1019 and the button 1001): like the design, the group box gives
; way to a "▶ <its caption>" label (static ${SKIN_LABEL_ID}, the caption's text and font) and under it the field and the
; button in one row, the button at the right edge, the field up to 10s before it, as tall as the button
Function ${UN}SkinFolderRow
  Push $0
  Push $1
  Push $2
  Push $3
  Push $4
  Push $5
  Push $6
  GetDlgItem $0 $SkPage 1020
  GetDlgItem $1 $SkPage 1019
  GetDlgItem $2 $SkPage 1001
  ${If} $0 P<> 0
  ${AndIf} $1 P<> 0
  ${AndIf} $2 P<> 0
    System::Call '*(i,i,i,i) p .r3'
    System::Call 'user32::GetClientRect(p $SkPage, p r3) i'
    System::Call '*$3(i, i, i .r6)'  ; page width
    System::Free $3
    Push $SkPage
    Push $0
    Call ${UN}SkinRectIn
    StrCpy $3 $SkY                   ; the group box's top
    ShowWindow $0 ${SW_HIDE}
    Push $0
    Call ${UN}SkinLineH
    StrCpy $5 $SkRH                  ; line height
    Push $SkCursorS
    Call ${UN}SkinBitmapSize
    IntOp $4 3 * $SkS
    IntOp $4 $4 + $SkBW              ; label left
    System::Call 'user32::GetWindowTextW(p r0, w .s, i ${NSIS_MAX_STRLEN}) i'
    Exch $SkPic
    IntOp $SkX $6 - $4
    ; WS_CHILD|WS_VISIBLE|SS_NOPREFIX
    System::Call 'user32::CreateWindowExW(i 0, w "STATIC", w "$SkPic", i 0x50000080, i r4, i r3, i $SkX, i r5, p $SkPage, p ${SKIN_LABEL_ID}, p 0, p 0) p .r4'
    Pop $SkPic
    SendMessage $0 ${WM_GETFONT} 0 0 $6
    SendMessage $4 ${WM_SETFONT} $6 0
    SetCtlColors $4 ${SKIN_TEXT} ${SKIN_CREAM}
    ; the row
    IntOp $3 $3 + $5
    IntOp $4 6 * $SkS
    IntOp $3 $3 + $4                 ; row top
    Push $SkPage
    Push $2
    Call ${UN}SkinRectIn
    System::Call '*(i,i,i,i) p .r4'
    System::Call 'user32::GetClientRect(p $SkPage, p r4) i'
    System::Call '*$4(i, i, i .r6)'
    System::Free $4
    IntOp $4 $6 - $SkRW              ; button left
    System::Call 'user32::SetWindowPos(p r2, p 0, i r4, i r3, i 0, i 0, i 0x15) i'
    IntOp $5 10 * $SkS
    IntOp $5 $4 - $5                 ; field width
    System::Call 'user32::SetWindowPos(p r1, p 0, i 0, i r3, i r5, i $SkRH, i 0x14) i'
  ${EndIf}
  Pop $6
  Pop $5
  Pop $4
  Pop $3
  Pop $2
  Pop $1
  Pop $0
FunctionEnd

; the components page (tree 1032, its label 1022 left of it, space required 1023 under the label): the label on a
; line of its own above the tree, the tree the page's full width (its items do not wrap: a long one needs the width),
; space required under the tree at the page's bottom
Function ${UN}SkinTreeRow
  Push $0
  Push $1
  Push $2
  Push $3
  Push $4
  Push $5
  Push $6
  Push $7
  GetDlgItem $0 $SkPage 1032
  GetDlgItem $1 $SkPage 1022
  GetDlgItem $2 $SkPage 1023
  ${If} $0 P<> 0
  ${AndIf} $1 P<> 0
  ${AndIf} $2 P<> 0
    System::Call '*(i,i,i,i) p .r3'
    System::Call 'user32::GetClientRect(p $SkPage, p r3) i'
    System::Call '*$3(i, i, i .r6, i .r7)'   ; page width, height
    System::Free $3
    Push $SkPage
    Push $1
    Call ${UN}SkinRectIn
    StrCpy $4 $SkY                         ; the row's top
    Push $1
    Call ${UN}SkinLineH
    StrCpy $3 $SkRH
    System::Call 'user32::SetWindowPos(p r1, p 0, i 0, i r4, i r6, i r3, i 0x14) i'
    Push $SkPage
    Push $2
    Call ${UN}SkinRectIn
    IntOp $5 $7 - $SkRH                    ; space required: at the bottom
    System::Call 'user32::SetWindowPos(p r2, p 0, i 0, i r5, i r6, i $SkRH, i 0x14) i'
    IntOp $4 $4 + $3
    IntOp $3 4 * $SkS
    IntOp $4 $4 + $3                       ; tree top
    IntOp $3 6 * $SkS
    IntOp $5 $5 - $3
    IntOp $5 $5 - $4                       ; tree height
    System::Call 'user32::SetWindowPos(p r0, p 0, i 0, i r4, i r6, i r5, i 0x14) i'
  ${EndIf}
  Pop $7
  Pop $6
  Pop $5
  Pop $4
  Pop $3
  Pop $2
  Pop $1
  Pop $0
FunctionEnd

; an inner page $0: a cream page, its texts dark on cream and tall enough for their text; the directory page's field
; row; every one-line field drawn like the design's path box (2s border ${SKIN_FIELD_LINE} on white, the field itself
; borderless inside it); the ▶ before the label
Function ${UN}SkinInner
  Push $1
  Push $2
  Push $3
  Push $4
  Push $5
  SetCtlColors $0 "" ${SKIN_CREAM}
  Call ${UN}SkinFolderRow
  Call ${UN}SkinTreeRow
  System::Call 'user32::GetWindow(p r0, i 5) p .r1'
  ${DoWhile} $1 P<> 0
    System::Call 'user32::GetClassNameW(p r1, w .r2, i 64) i'
    System::Call 'user32::GetWindowLongW(p r1, i -16) i .r3'
    IntOp $4 $3 & 0x10000000         ; WS_VISIBLE
    IntOp $3 $3 & 0x1F
    ${If} $2 == "Static"
      SetCtlColors $1 ${SKIN_TEXT} ${SKIN_CREAM}
      ; texts (SS_LEFT, SS_CENTER, SS_RIGHT) that are shown and have something in them
      System::Call 'user32::GetWindowTextLengthW(p r1) i .r5'
      ${If} $3 <= 2
      ${AndIf} $4 <> 0
      ${AndIf} $5 > 0
        Push $1
        Call ${UN}SkinFit
      ${EndIf}
    ${ElseIf} $2 == "Button"
    ${AndIf} $3 >= 2
    ${AndIf} $3 <= 9
      ; check boxes, radio buttons, group boxes (not push buttons)
      SetCtlColors $1 ${SKIN_TEXT} ${SKIN_CREAM}
      ; a shown check box or radio button (not a group box, 7) wraps its text and is as tall as it needs
      ${If} $3 != 7
      ${AndIf} $4 <> 0
        Push $1
        Call ${UN}SkinFitBox
      ${EndIf}
    ${EndIf}
    System::Call 'user32::GetWindow(p r1, i 2) p .r1'
  ${Loop}
  Call ${UN}SkinOverflow
  ; the page's own picture: only when it has a field or the label to draw
  StrCpy $5 0
  FindWindow $1 "Edit" "" $0
  ${If} $1 P<> 0
    StrCpy $5 1
  ${EndIf}
  GetDlgItem $1 $0 ${SKIN_LABEL_ID}
  ${If} $1 P<> 0
    StrCpy $5 1
  ${EndIf}
  ${If} $5 = 1
    Push $0
    Call ${UN}SkinBegin
    System::Call '*(i 0, i 0, i $SkW, i $SkH) p .r2'
    System::Call 'gdi32::CreateSolidBrush(i ${SKIN_CREAM_REF}) p .r3'
    System::Call 'user32::FillRect(p $SkDC, p r2, p r3) i'
    System::Call 'gdi32::DeleteObject(p r3) i'
    System::Free $2
    System::Call 'user32::GetWindow(p r0, i 5) p .r1'
    ${DoWhile} $1 P<> 0
      System::Call 'user32::GetClassNameW(p r1, w .r2, i 64) i'
      System::Call 'user32::GetWindowLongW(p r1, i -16) i .r3'
      IntOp $4 $3 & 0x10000000
      IntOp $3 $3 & 0x4              ; ES_MULTILINE
      ${If} $2 == "Edit"
      ${AndIf} $3 = 0
      ${AndIf} $4 <> 0
        Call ${UN}SkinField
      ${EndIf}
      System::Call 'user32::GetWindow(p r1, i 2) p .r1'
    ${Loop}
    GetDlgItem $1 $0 ${SKIN_LABEL_ID}
    ${If} $1 P<> 0
      Push $0
      Push $1
      Call ${UN}SkinRectIn
      Push $SkCursorS
      Call ${UN}SkinBitmapSize
      IntOp $2 $SkRH - $SkBH
      IntOp $2 $2 / 2
      IntOp $SkY $SkY + $2
      StrCpy $SkX 0
      Call ${UN}SkinCursorS
    ${EndIf}
    Push $0
    Call ${UN}SkinDone
  ${EndIf}
  Pop $5
  Pop $4
  Pop $3
  Pop $2
  Pop $1
FunctionEnd

; the field $1 on the page $0 (drawing into $SkDC): its place becomes the path box (border, white), the field itself goes
; inside it without its own border, one line tall, centred
Function ${UN}SkinField
  Push $2
  Push $3
  Push $4
  Push $5
  Push $6
  Push $0
  Push $1
  Call ${UN}SkinRectIn
  ; the box: border colour all over, white inside the 2s border
  IntOp $2 $SkX + $SkRW
  IntOp $3 $SkY + $SkRH
  System::Call '*(i $SkX, i $SkY, i r2, i r3) p .r4'
  System::Call 'gdi32::CreateSolidBrush(i ${SKIN_FIELD_LINE_REF}) p .r5'
  System::Call 'user32::FillRect(p $SkDC, p r4, p r5) i'
  System::Call 'gdi32::DeleteObject(p r5) i'
  IntOp $5 2 * $SkS
  System::Call 'user32::InflateRect(p r4, i -$5, i -$5) i'
  System::Call 'gdi32::CreateSolidBrush(i 0x00FFFFFF) p .r5'
  System::Call 'user32::FillRect(p $SkDC, p r4, p r5) i'
  System::Call 'gdi32::DeleteObject(p r5) i'
  System::Free $4
  ; the field: no border of its own (WS_EX_CLIENTEDGE, WS_BORDER), 9s in from the box's sides, one line tall, centred
  System::Call 'user32::GetWindowLongW(p r1, i -20) i .r4'
  IntOp $4 $4 & 0xFFFFFDFF
  System::Call 'user32::SetWindowLongW(p r1, i -20, i r4) i'
  System::Call 'user32::GetWindowLongW(p r1, i -16) i .r4'
  IntOp $4 $4 & 0xFF7FFFFF
  System::Call 'user32::SetWindowLongW(p r1, i -16, i r4) i'
  StrCpy $2 $SkX
  StrCpy $3 $SkY
  StrCpy $4 $SkRW
  StrCpy $6 $SkRH
  Push $1
  Call ${UN}SkinLineH
  IntOp $5 9 * $SkS
  IntOp $2 $2 + $5
  IntOp $4 $4 - $5
  IntOp $4 $4 - $5
  IntOp $5 $SkRH + $SkS
  IntOp $5 $5 + $SkS                 ; field height: a line and 1s above and below
  IntOp $6 $6 - $5
  IntOp $6 $6 / 2
  IntOp $3 $3 + $6
  ; SWP_FRAMECHANGED|SWP_NOZORDER|SWP_NOACTIVATE
  System::Call 'user32::SetWindowPos(p r1, p 0, i r2, i r3, i r4, i r5, i 0x34) i'
  SetCtlColors $1 ${SKIN_TEXT} FFFFFF
  Pop $6
  Pop $5
  Pop $4
  Pop $3
  Pop $2
FunctionEnd

; every page's SHOW: hide MUI's own header background, header picture, lines and etched brand text; dress the page;
; then the main window
Function ${UN}SkinPage
  ${If} $SkS != ""
    Call ${UN}SkinFreePages
    Push $0
    !insertmacro SKIN_HIDE 1034
    !insertmacro SKIN_HIDE 1046
    !insertmacro SKIN_HIDE 1035
    !insertmacro SKIN_HIDE 1036
    !insertmacro SKIN_HIDE 1045
    !insertmacro SKIN_HIDE 1028
    ; the page: at SHOW it is not visible yet; the page before it may still be there (custom pages: until
    ; nsDialogs::Show), so take the hidden one, else the first
    Push $1
    Push $2
    StrCpy $0 0
    FindWindow $1 "#32770" "" $HWNDPARENT
    StrCpy $2 $1
    ${DoWhile} $1 P<> 0
      System::Call 'user32::IsWindowVisible(p r1) i .r0'
      ${If} $0 = 0
        StrCpy $0 $1
        ${Break}
      ${EndIf}
      StrCpy $0 0
      FindWindow $1 "#32770" "" $HWNDPARENT $1
    ${Loop}
    ${If} $0 P= 0
      StrCpy $0 $2
    ${EndIf}
    Pop $2
    Pop $1
    StrCpy $SkPage $0
    Push $0
    Push 0
    Call ${UN}SkinRect
    ${If} $SkX > 0
      Call ${UN}SkinInner
    ${Else}
      Call ${UN}SkinWelcome
    ${EndIf}
    ; the main window's own texts (header title and subtext, brand): measured for the tests, their places are fixed
    Call ${UN}SkinHeader
    !insertmacro SKIN_MEASURE 1256
    Call ${UN}SkinMain
    ; RDW_INVALIDATE|RDW_ERASE|RDW_ALLCHILDREN|RDW_UPDATENOW
    System::Call 'user32::RedrawWindow(p $HWNDPARENT, p 0, p 0, i 0x185) i'
    Pop $0
  ${EndIf}
FunctionEnd
!macroend
!insertmacro SKIN_FUNCS "" ""
!insertmacro SKIN_FUNCS "un." "UN"

; Modern UI has no GUIEND hook of its own: the plain NSIS callbacks (not called in silent mode either)
Function .onGUIEnd
  Call SkinEnd
FunctionEnd
Function un.onGUIEnd
  Call un.SkinEnd
FunctionEnd
