# DBZ 超賽亞傳說 繁中版：單檔 Windows 啟動器

把上游 `ssd-recompiled.exe`、繁中譯文、字型和授權檔包成**一個 exe**，雙擊就能玩，不需要 bat，也不需要手動放 tsv 或字型。

## 執行流程

1. **解開**：嵌入的檔案解開到 `%LOCALAPPDATA%\dbz-ssd-zhtw\app\<BUNDLE_ID>\`。`BUNDLE_ID` 是全部嵌入檔 sha256 的前 12 碼，所以內容沒變就沿用舊資料夾、不重新解開；內容一變就換一個新資料夾。解開時先寫到暫存資料夾，最後寫入 `.unpacked` 標記、再改名成正式資料夾，所以不會用到寫了一半的檔案。
1a. **資料目錄與第一次遷移**（每次啟動，`resolve_data_dir`／`migrate`）：
   - 資料目錄＝啟動器 exe 旁的 `save\`，啟動遊戲時以環境變數 `SSD_DATA_DIR` 傳給上游（上游 `crates/ssd-app/src/paths.rs` 的 `data_dir`：`saves\ssd.sav` 與背包／作弊／選項附屬檔、`states\slotN.ssdstate`、`cache\rom\canonical.sfc`、譯文與字型都在它底下）。
   - 使用者自己已設 `SSD_DATA_DIR`：沿用，不覆蓋，也不遷移。
   - `save\` 不存在就建立，並寫一個探測檔確認寫得進去；建不了或寫不進去（唯讀位置）就退回上游預設 `%LOCALAPPDATA%\dbz-ssd-recompiled`。用了哪個、為什麼，寫在 `last-run.log` 開頭與 selftest 輸出（`data_dir … (beside the launcher|SSD_DATA_DIR set by the player, kept|fallback: …)`）。
   - 遷移：資料目錄是 `save\`、`save\saves\ssd.sav` 不存在、而 `%LOCALAPPDATA%\dbz-ssd-recompiled\saves\` 存在時，把舊目錄的 `states\`（含 `slotN.hd`）、`mods\`（上游 0.3.x 的資料包）、`cache\mods\`、`cache\rom\canonical.sfc`、`saves\`（含 `ssd.mods`）整個複製過去，`saves\ssd.sav` 最後經 `.part` 檔改名寫入（有它就代表複製完整）。只複製：舊目錄不刪、不改名；`save\` 裡已有的檔不覆蓋。log 記 `migrated from …`；複製失敗跳訊息框並結束，不開遊戲。
   - `cache\rom\canonical.sfc` 一併複製，就不必再選一次 ROM。
   - **清理舊檔**（遷移完成或不需要之後，`clean_up`）：
     - 舊解包版本：`%LOCALAPPDATA%\dbz-ssd-zhtw\app\` 底下不是現行 `BUNDLE_ID` 的資料夾全部刪除（另一份啟動器正在寫的 `<現行 id>.tmp-…` 除外）。刪不掉（檔案被占用）就略過、記 log，下次啟動再試。
     - 舊資料目錄 `%LOCALAPPDATA%\dbz-ssd-recompiled\`：只在資料目錄是 exe 旁 `save\` 時處理。`save\saves\ssd.sav` 存在，而且舊目錄 `saves\`、`states\`、`mods\` 底下每個檔在 `save\` 都有、sha256 相同（`cache\` 不比，遊戲會重建），才刪舊目錄的 `saves\`、`states\`、`mods\` 三個子目錄，其他（`cache\`、譯文、字型、上游原版自己的檔）留著；任何一個檔對不上就一個都不刪，log 寫明是哪個檔。使用者自己設了 `SSD_DATA_DIR`、或退回預設目錄時都不碰它。
     - 刪除一律先改名成 `<資料夾>.removing` 再刪：刪到一半失敗只會留下 `.removing`，不會留下半個原名資料夾，下次啟動會把它刪完。
     - 刪了哪些、略過哪些與原因都寫在 `last-run.log` 開頭（`cleanup removed|skipped|kept …`）。這幾列在清理完就寫入，遊戲的輸出接在後面；選 ROM 時按取消也留得住。
1b. **同步譯文到資料目錄**（每次啟動）：台版 `zh-tw.tsv`、`ui-zh-tw.tsv`，港版 `zh-hk.tsv`、`ui-zh-hk.tsv`，`fonts\LXGWWenKaiTC-Regular.ttf`、`fonts\OFL.txt` 寫進 1a 決定的資料目錄（通常是 exe 旁的 `save\`）。早期啟動器在 `zh-hk` 代碼下留在資料目錄的台版表，也照下面的規則換成港版表、舊檔留 `.bak`。內容相同就跳過（`sync unchanged`）；譯文表只差 `#font` 那一列、而且是玩家改過的（不是啟動器上次寫的那份：資料目錄的 `.launcher-sync.toml` 記著每張表上次寫入的 sha256；還沒有這個檔時，較早版本內附的兩張表〔`KNOWN_TABLES`〕也算啟動器的）時，保留玩家的 `#font`、以這版的表加玩家的 `#font` 寫回，不產生 `.bak`（`sync kept-font`，`sync_into`／`font_kept`）；只差換行或 BOM 而 `#font` 相同時算 `sync unchanged`；其他不同就先把舊檔改名為 `<檔名>.bak` 再覆蓋（`sync replaced (old kept as .bak)`）。每個檔的結果都寫進 `last-run.log`。上游先讀資料目錄，這一步確保它讀到的是這版譯文。寫入失敗會跳訊息框並結束。
   - **同步之前：舊玩家的語言**（每個資料目錄只做一次，在同步之前，`settle_language`）：舊版啟動器每次都用 `--lang zh-tw` 開遊戲，蓋掉存檔裡的語言；這版不再傳 `--lang`。所以這次啟動**之前**資料目錄已有 `.launcher-sync.toml`、或 `saves\` 裡有檔的玩家，若 `saves\ssd.settings` 的 `access.language` 不是 `zh-tw`（沒有這一列、`en`、早期版本的數字索引、`zh-hk`、其他語言碼），就改成 `zh-tw`（已經是 `zh-tw` 就不動檔案）。`zh-hk` 也改，是因為這版之前沒有真正的港版表：舊玩家存的 `zh-hk` 只會是早期啟動器把台版表放在 `zh-hk` 代碼下、或第一版的索引 1 被遊戲轉成的，實際玩的都是台版。只玩過上游英文版、存檔被搬進資料目錄的人也算舊玩家，直接定成台版、不出語言頁，仍可到選項裡切語言。改寫時只動這一列（每列的換行照舊），沒有就加在檔尾，其他位元組不變；沒有這個檔就建立只含 `ssd-settings 1` 與這一列的檔；先寫 `ssd.settings.tmp` 再改名。新安裝的資料目錄不動，由遊戲第一次啟動的語言頁讓玩家選台灣或香港。做完寫標記 `.launcher-lang`，之後玩家怎麼選都不再改。設定檔格式不對（不是文字、或第一行不是 `ssd-settings N`）時不改它、但照樣寫標記，log 記 `language left to the game`：第一行不對的，遊戲會把它移成 `.bad` 並出語言頁；不是文字的，遊戲讀不進去、用預設值。之後玩家選的語言是明確的選擇，不會再被改（設定檔壞掉的舊玩家因此會看到語言頁）。讀寫失敗（讀檔 I/O 錯誤、寫 `.tmp` 失敗、改名失敗，例如防毒鎖檔）時不寫標記，記 `language settle FAILED`，下次再試：這時舊值（`en` 或數字索引）仍有效、又沒有 `--lang` 蓋過，不重試舊玩家會一直看到英文；代價是在失敗到下次成功之間，玩家若自己到選項裡改了語言，會被改回台版一次。已知限制：新安裝而標記寫不進去時，下次啟動（同步已寫出 `.launcher-sync.toml`）會被當成舊玩家、語言被定成台版，直到標記寫得進去為止，log 記 `language marker FAILED`。另一個已知限制：舊玩家的設定檔長期讀不了或寫不進去（例如被鎖住）時，啟動器每次啟動都會重試並記在 log（`language settle FAILED`），這段期間遊戲用的是設定檔裡原本的語言。結果寫進 `last-run.log`（`language …`）。
1c. **超級達爾去墊肩美術**（每次啟動，`set_up_art_override`／`install_art_override`）：嵌入的 `art-override/battle-super-vegeta-NNN.png`（Ming 以 Google Gemini 圖像模型編輯自 HD 美術包原圖，CC BY-NC-SA 4.0）寫進資料目錄的 `art-override\`；遊戲用 `<資料目錄>\art-override\<art id>.png` 取代美術包同名、同尺寸的圖，照樣套色表、受擊閃色與淡入淡出（尺寸不同的圖不用，log 寫原因）。另有子資料夾 `art-override/super-vegeta/<art id>.png`：美術包只有普通達爾圖的超級達爾姿勢（能量波、被踢飛等），寫進 `art-override\super-vegeta\`，遊戲在畫面是超級達爾配色（含金髮）時才用，普通達爾照舊用美術包的圖。另有 Mode 7 平面的子資料夾 `art-override/mode7/<content key>/`（兩層，遊戲 `ssd_hd::pack::MODE7_DIR`）：寫進 `art-override\mode7\<content key>\`，含 PNG 圖塊 `t<x>_<y>.png`、可選的 `palette.txt` 與可選的空檔 `replace`。`art_override_pictures` 取根目錄與 `super-vegeta/`、`goku/`、`variants/` 一層的 PNG，以及 `mode7/<content key>/` 的 PNG、`palette.txt`、`replace`，與 `super-vegeta/` 裡檔名是 `key-<16 位十六進位鍵值>.txt` 的像素檔（遊戲讀它核對同名的 `key-<鍵值>.png`：美術包沒有美術的超級達爾姿勢）；更深的資料夾與其他檔不取。只寫一次：資料目錄的 `.launcher-art`（在 `art-override\` 旁邊，不在裡面）記著啟動器寫過的每張圖與 sha256，玩家刪掉或改過的圖之後不再寫回（log 記 `kept removed`／`kept (changed by the player)`）；新版啟動器附的圖只取代仍是啟動器上次寫的那份。原本就有、內容不同的同名檔（玩家自己的圖）不覆蓋、也不記。想恢復原本的墊肩：刪掉 `save\art-override\` 整個資料夾即可（`.launcher-art` 在資料夾外，啟動器不會再寫回）；同一資料夾附 `LICENSE-ART.txt`（出處、授權、改作者）。使用者自己設了 `SSD_DATA_DIR` 時不做。寫入失敗只記 `art override FAILED` 到 log，照常開遊戲。結果一列寫進 `last-run.log`（`art override …: N installed, N updated, N same`）。
2. **署名框**：第一次執行時跳一次「DBZ 超賽亞傳說 繁中版　繁體中文化 By Ming Liu／本程式基於 JackRushante/DBZ-Super-Saiya-Densetsu-Rust（GPL-3）」，之後不再跳（標記檔：`%LOCALAPPDATA%\dbz-ssd-zhtw\.credits-shown`）。
3. **ROM 設定**（只在快取的 ROM `<資料目錄>\cache\rom\canonical.sfc` 不存在時做）：
   - 資料目錄見 1a（通常是 exe 旁的 `save\`）；ROM 設定與開遊戲時都帶 `SSD_DATA_DIR`，上游寫快取的位置就是啟動器檢查的位置。
   - ROM：exe 檔尾有 overlay `rom` 段就直接用它（先寫到 `%TEMP%\dbz-ssd-zhtw-setup-<pid>\`，設定完就刪除）；沒有的話跳檔案選擇框（.sfc／.smc），按取消就結束程式。
   - ips：依序用 overlay 的 `ips` 段、編譯時嵌入的 `dbz102.ips`（`bundle-ips`），都沒有就跳選擇框（英文版 ROM 按取消）。
   - 執行 `ssd-recompiled.exe --rom <rom> [--ips <ips>] --headless-frames 1`。失敗時用訊息框顯示上游 stderr 的最後 1500 字。
4. **開遊戲**（存檔寫進 1a 的資料目錄）：在解開的資料夾內執行 `ssd-recompiled.exe --mode enhanced --hd-scale 4`（不傳 `--lang`：語言由玩家在遊戲裡選，存在 `saves\ssd.settings`；`--lang` 會每次蓋掉玩家的選擇）。若本啟動器旁邊有 `hd-pack.zip`、`hd-pack` 資料夾，或檔名含 `hd-pack` 的 zip，就再加 `--install-hd-pack <路徑>`。這個參數重複帶不會重裝：上游比對 manifest 的 digest 後，已經裝過就直接回傳 `Current`。啟動前先讀包內 `hd-pack.toml` 的 `digest`／`revision`（`hd_pack_check`）：不是 v0.3.9 的 `0EF46E0B`／`2`（`hd-pack.zip` 484,121,119 位元組）就在 `last-run.log` 記一列「HD 美術包不是 v0.3.9，超賽悟空豎髮與頭像修正不會生效，請更新」，並跳一次提示框（同一個包只跳一次，記在 `%LOCALAPPDATA%\dbz-ssd-zhtw\.hd-pack-warned`），然後照常開遊戲。上游是 console 程式，所以用 `CREATE_NO_WINDOW` 啟動，不會跳黑窗；stdout／stderr 寫到 `%LOCALAPPDATA%\dbz-ssd-zhtw\last-run.log`，非 0 結束時跳訊息框。

### 上游怎麼找語言檔與字型（依據）

- `crates/ssd-app/src/lang.rs`（上游 `41f86b3` 的語言包機制，PR #13）：語言碼 `zh-tw` 的檔名是 `table_file` = `zh-tw.tsv`、`ui_file` = `ui-zh-tw.tsv`；搜尋順序（`places()`）是先找**資料目錄**，再找 **exe 所在資料夾**。字型在這兩處的 `fonts\` 底下找，`zh-tw.tsv` 檔頭 `#font<TAB>NotoSansTC` 指定的 `NotoSansTC*` 優先，`-Regular` 優先。
- `crates/ssd-app/src/main.rs`：`--lang`（`en` 或任何語言碼，上游的語言包機制）、`--mode`（`classic`／`enhanced`）、`--hd-scale`、`--install-hd-pack`、`--symbols`（預設 `--hooks original` 時不讀 symbols.toml）。

因為資料目錄優先，啟動器每次都把譯文同步到資料目錄（見 1b），玩家以前手動放進去的舊檔會被覆蓋（留 `.bak`）。字型照 `zh-tw.tsv` 檔頭 `#font` 取 `LXGWWenKaiTC*`（霞鶩文楷 TC；更早的版本用 `NotoSansTC*`）。以前同步進資料目錄的 `fonts\NotoSansTC-Regular.otf` 留著不刪，`#font` 指定的字型排在它前面。

### 存檔位置（給玩家）

存檔位置：exe 旁的 `save\` 資料夾（電池存檔 `saves\`、即時存檔 `states\`、ROM 快取 `cache\` 都在裡面）。從雲端同步資料夾直接執行時，存檔也會被同步，同步中偶爾會鎖住檔案，建議把 exe 放在本機資料夾（例如 `C:\Games\DBZ\`）再玩。

## exe 檔尾的附加資料（overlay）

啟動器會讀 exe 檔尾的附加資料（格式見 `src/overlay.rs`），公開版沒有附加資料。

## `--selftest`

解開 assets、逐檔重算 sha256、執行並列出 1a 的最終資料目錄與遷移結果（`data_dir …`、`migrated from …`／`migration not needed|none|skipped …`；**會實際建立 `save\` 並遷移**）、清理的「將刪除」清單（`cleanup would remove …`／`cleanup kept …`；**selftest 不刪任何東西**）、1b 的舊玩家語言處理與同步結果（`language …`、`sync same|written|replaced`；**會實際寫入資料目錄**）、1c 的去墊肩美術會做什麼（`art override …: N would be installed …`；**不寫入**）、列出路徑、資料目錄、ROM 快取是否存在、overlay 有無與各段 sha256，以及 hd-pack 位置。不跳任何視窗，也不啟動遊戲；全部通過 exit 0，否則 exit 1。結果同時寫進 `%LOCALAPPDATA%\dbz-ssd-zhtw\selftest.txt`。

## 自動更新（目前的發行版未啟用）

目前的發行版建置時沒有設更新來源，這一節的功能整個不啟動：不連網、不檢查、不提示新版。以下說明設了更新來源的建置會怎麼做。

- 建置時設了更新來源（下面「Build」的 `DBZ_UPDATE_MANIFEST_URL` 與 `DBZ_UPDATE_PUBLIC_KEY`）時，啟動器一開啟就在背景讀已簽章的更新清單，**一天最多一次**（逾時 4 秒、最多 256 KB；失敗不提示，只寫進 `last-run.log`，下次開啟再試）。**開始遊戲不等這個檢查**：解包完成時檢查已經回來，就在開始遊戲前提示；還沒回來的話，結果存在 `update-offer.txt`，下次開啟時重新驗簽後再提示。
- 提示有「立即更新」「稍後」兩個按鈕。按「立即更新」會開一個進度視窗：進度條、已下載／總大小（MB）、百分比，可以按「取消」；取消後刪掉下載到一半的 `.update.tmp`，舊 exe 不動。下載邊收邊核對大小與 sha256，接上本 exe 檔尾的 overlay（自備 ROM 版的 ROM／IPS，原樣保留，不上傳），再**從磁碟重讀一次核對**，這時顯示「驗證中…」；接著把舊 exe 改名成 `<檔名>.previous.exe`、新 exe 放回原檔名，顯示「安裝更新中…」；換好後再從磁碟核對一次新 exe 的 sha256 與 overlay（暫存檔在核對與改名之間沒有被鎖住，可能被別的程式換掉），不符就換回舊版；通過才開啟新版。驗證與換檔階段不能取消；取消按得太晚（下載剛完成）時，畫面直接顯示實際階段，更新照常完成。下載卡住時按取消，最久要等 30 秒閒置逾時才會停下，結果算取消、不跳錯誤框。下載完成後視窗會自己關閉。新版成功開啟後才刪掉 `.previous.exe`。任何一步失敗都維持原版。
- 下載 30 秒收不到資料就算失敗（總上限 2 小時）。新 exe 改名放回時若被防毒暫時佔住，會隔 0.3 秒重試，最多 5 次，再不行就換回舊版。下載用的暫存檔只能由一個啟動器建立：同時開兩個時，第二個會說「另一個視窗正在下載同一個檔」；被強制結束留下的暫存檔，下次開啟時刪掉。
- 進度視窗與自訂按鈕用 Windows 的 TaskDialog（comctl32 v6，exe 內嵌 manifest 選用它）。它是在執行時查找的，而且不會從 exe 所在資料夾載入任何 comctl32：已載入的必須位於 Windows 資料夾底下；沒載入時，只有在 exe 的 manifest 確實把 comctl32.dll 重導到 v6（`FindActCtxSectionStringW`）時才用名稱載入，此時系統從 WinSxS 取用、不搜尋任何資料夾。查不到時（manifest 沒生效），問題改用「是／否」對話框，下載在背景照常進行、不顯示進度，結束後用訊息框告知結果。
- 只接受 https；清單只從建置時設的那個主機讀；下載只接受清單主機與 GitHub release 的 `github.com`、`objects.githubusercontent.com`、`release-assets.githubusercontent.com`，每一次轉址都會檢查。清單要用內建公鑰（ed25519）簽過，版本不比目前新就不提示（防降版）。請求只有固定的 User-Agent `dbz-zhtw-updater/<NN>`，不帶任何玩家資料。TLS 用系統的（Windows 是 SChannel）。
- 玩家設定在 `%LOCALAPPDATA%\dbz-ssd-zhtw\update-settings.txt`：`check_updates = false` 就不檢查、不連網、也不提示；按「稍後」的版本記在 `skipped_version`，不再提示那一版。上次檢查的時間在 `update-checked.txt`，刪掉它下次開啟就會再檢查。
- **兩個設定少一個（或都沒設）時，整個功能不啟動：不連網，也不顯示任何更新畫面。**
- `--selftest` 多一行 `release <名稱> (<NN>) update_source <網址|none> check_updates <true|false>`，不連網。

## HD 美術包詢問

exe 旁沒有 HD 美術包時，開始遊戲前會問要不要下載上游 v0.3.9 的 `hd-pack.zip`（上游作者的作品，CC BY-NC-SA 4.0，不是本專案內容），按鈕是「下載」「這次不要」（下次開啟再問；關掉視窗也算這個）「不再詢問」。沒有 TaskDialog 時改用「是＝下載／否＝不再詢問／取消＝這次不要」。回答記在 `%LOCALAPPDATA%\dbz-ssd-zhtw\hd-pack-offer.txt`。按「下載」之前不發任何請求。下載用和更新相同的進度視窗與取消，先從上游 GitHub release 讀；上游拿不到檔（連不上、404／410／5xx、逾時或中途斷線、轉址到不允許的主機、內容的大小或 sha256 不對）時自動改從備援網址讀（`hd_offer::MIRROR_URL`：我方公開 fork 的 Release `hd-pack-v0.3.9`，同一個檔），`.part` 清空從頭寫，核對與上游完全相同；玩家取消、另一個啟動器正在下載、寫檔失敗（磁碟滿、沒權限）都不改抓備援。兩邊都失敗時訊息列出兩邊各自的原因。兩個來源都只走同上三個主機，最多 484,121,119 位元組，sha256 必須等於內建值 `e55cb05d…21d4cc`（GitHub API 公布的 asset digest，與既有副本相同）；邊收邊核對，再從磁碟重讀核對一次，通過後才把 `hd-pack.zip.part` 改名成 exe 旁的 `hd-pack.zip`（被防毒暫時佔住會重試），改名後再核對一次；不通過、取消、或 30 秒收不到資料就刪掉。我方的 `art-override\` 改圖不受影響（遊戲用它取代美術包同名的圖）。這個詢問不需要設定更新來源。

`--hd-pack`：給選過「這次不要」或「不再詢問」後又想要美術包的人。不開始遊戲，不管先前回答過什麼都先問一次（「下載」「取消」；沒有 TaskDialog 時是「確定／取消」），按下載才連網，下載與上面相同（進度視窗、取消、sha256、磁碟重驗、同時只有一個啟動器下載），結束時用訊息框告知結果。exe 旁已有 v0.3.9 的包（`hd_pack_check` 認可）就只告知已經有了；有的不是 v0.3.9：別的檔名（例如 `hd-pack-v0.3.8.zip`）不動，新的 `hd-pack.zip` 優先使用；`hd-pack.zip` 本身不是 v0.3.9 時，新檔通過核對後先把它改名成 `hd-pack.zip.old`（被佔用就用 `.old2`、`.old3`……，不覆蓋任何檔；副檔名不是 `.zip`，啟動器的 `hd_pack_in` 與上游 `install::find_source` 都不會把它當美術包），再放新檔並在原處再核對一次；改名失敗就不放新檔，核對失敗就刪掉新檔、把舊檔改回原名。備份檔啟動器不會自動刪除。下載成功後把 `hd-pack-offer.txt` 改成 `download`。`--help` 用訊息框（同時印到 stdout）列出所有參數。

## Build（Windows）

1. 準備 assets：`./fetch_assets.sh <來源資料夾>`（zsh；也可以改設環境變數 `DBZ_ASSETS_SRC`；要開 `bundle-ips` 時加 `--with-ips`）。來源資料夾要放哪些檔案見 `assets/README.md`。
2. 在任一台有 Rust toolchain（rustup；版本由 `rust-toolchain.toml` 指定，1.99）＋ Visual Studio C++ build tools（版本資訊資源需要 VS 的 `rc.exe`）的 Windows 機器上，於本目錄載入 vcvars64 後執行 `cargo build --release`。也可以用 `powershell -NoProfile -ExecutionPolicy Bypass -File build_windows.ps1 build`：腳本會先載入 vcvars64，並以 `-j 1`、BelowNormal 優先權執行（共用機器時降低優先權）；cargo 不在 `%USERPROFILE%\.cargo\bin` 時加 `-CargoBin <資料夾>`。
3. 產出 `target\release\dbz-zhtw-launcher.exe`。安裝程式把它裝成 `DBZ_超賽亞傳說_繁中.exe`（見 `../installer/README.md`）。

Cargo 的 bin 名稱用 ASCII（`dbz-zhtw-launcher`），中文檔名在複製時才改；版本資訊的 OriginalFilename 已經是中文檔名。

### 版號與更新來源（目前的發行版沒有設更新來源）

目前的發行版建置時沒有設下面兩個環境變數，建出的是不連網、不提示新版的版本（見上面「自動更新（目前的發行版未啟用）」）。以下的更新來源設定與簽清單步驟，只適用於要啟用自動更新的建置。

- 版本的數字寫在 `RELEASE`（一個整數），啟用自動更新時拿它比新舊；畫面上顯示的版本名稱（例如 RC1）在 `RELEASE_NAME`；Cargo 的 `version` 仍是 1.0.0，`BUNDLE_ID` 用途不變。
- 要讓建出的 exe 會自動更新，建置時設兩個環境變數（兩個都要，少一個就建出不連網的版本，cargo 會印 warning）：
  - `DBZ_UPDATE_MANIFEST_URL`：已簽章清單的 https 網址（例如放在 GitHub release 的 `https://github.com/<帳號>/<repo>/releases/latest/download/update-manifest.txt`）。
  - `DBZ_UPDATE_PUBLIC_KEY`：簽章公鑰，64 個十六進位字元（`dbz-update-sign pubkey <私鑰檔>` 印出）。
  - 有設但格式不對時建置直接失敗。
- 簽清單：`cargo run --features signer --bin dbz-update-sign -- sign --exe <公開版 exe> --version <NN> --url <該 exe 的 https 下載網址> --key <私鑰檔> --out update-manifest.txt`（exe 不放在 GitHub release、而放在清單同一個主機時，加 `--manifest-url <DBZ_UPDATE_MANIFEST_URL>`；其他主機的網址會被拒簽，因為啟動器不會從那裡下載）。私鑰檔是 64 個十六進位字元（例如 `openssl rand -hex 32 > key.txt`），只從參數指定的路徑讀，**不要放進 repo**；私鑰遺失後，已發出去的 exe 就再也收不到更新。帶 overlay（ROM／IPS）的 exe 會被拒簽。清單格式見 `src/update_manifest.rs`。
- 啟用自動更新的版本每次發版：建 exe → 確認 exe 檔尾沒有附加資料 → 把 exe 本身（不是壓縮檔）傳成 release 附件 → 用附件網址簽清單 → 把清單也傳成同一個 release 的附件。
- 建置時 exe 會內嵌 application manifest（comctl32 v6 與 asInvoker，`build.rs` 的 `MANIFEST`），由 winresource 和版本資訊一起編進去。

### feature flag

- `bundle-ips`（預設關）：把 `assets/dbz102.ips` 嵌進程式本體。overlay 的 ips 優先於它。公開版不開這個 flag。

## 署名與授權

- 署名「繁體中文化 By Ming Liu」出現在兩處：exe 版本資訊（ProductName「DBZ 超賽亞傳說 繁中版」、FileDescription「繁體中文化 By Ming Liu」、LegalCopyright 寫上游 GPL-3 聲明＋「繁中譯文 © Ming Liu」）與首次執行的署名框。HD 選項語言列的說明另有譯者署名，在譯文表裡：台版「繁體中文（台灣）翻譯：Ming Liu」、港版「繁體中文翻譯：Ming Liu」。
- 上游 ssd-recompiled（JackRushante/DBZ-Super-Saiya-Densetsu-Rust）與其連結的 jgenesis 都是 **GPL-3.0**。本啟動器以同一授權發布（`license = "GPL-3.0-or-later"`），並把上游 `LICENSE.txt`、`NOTICE.md`、`licenses/` 一起嵌入、一起解開。GPL-3 要求保留上游著作權聲明：我們的署名**只加不減**，不得移除或取代上游聲明；散布 binary 時須附上對應原始碼（上游 repo＋本 repo）。
- HD 美術包（`hd-pack.zip`）是 **CC BY-NC-SA 4.0**（上游 `LICENSE-ART.md`），不嵌入，由玩家自備、放在 exe 旁。
- 覆蓋圖裡改作自 HD 美術包原圖的那些圖（超級達爾去墊肩等，在 `art-override/` 與其子資料夾）：Ming 以 Google Gemini 圖像模型編輯自 HD 美術包原圖，依原圖授權以 **CC BY-NC-SA 4.0** 散布，嵌入啟動器。`gallery-party-piccolo-u01-061.png`、`variants/battle-goku-ssj-020.png`、`super-vegeta/key-*.png` 與 `ssj-lightning/` 除外，見 `LICENSE-ART.txt`。
- 字型霞鶩文楷 TC（LXGW WenKai TC 1.530）是 **SIL OFL 1.1**（`fonts/OFL.txt` 一起附上）；上游介面字型的 OFL 授權在 `licenses/`。
- `dbz102.ips`（Klepto v1.02 英文補丁）與遊戲 ROM 都**不是我們能散布的東西**，公開版兩者都不含。

## 第一次設定與 ROM 錯誤

### 預放的 ROM 與 IPS（給安裝程式與進階使用者）

- 第一次啟動（資料夾裡還沒有 `cache\rom\canonical.sfc`）時，啟動器先看 exe 旁的 `save\setup\`：有 `rom.sfc`（或 `rom.smc`，依原檔的副檔名）就拿它，連同同一個資料夾裡選填的 `dbz102.ips`，走和自己選檔完全相同的步驟（日版 Rev 1 缺 IPS 的檢查、讓遊戲跑一幀產生 `canonical.sfc`），不跳「第一次執行：請選擇遊戲 ROM」與兩個選檔對話框。安裝程式就是把玩家選的檔**複製**到這裡；也可以自己放。
- 只在 exe 旁的 `save\setup\` 找，不管資料夾是否因 `SSD_DATA_DIR` 或 `save\` 寫不進去而改用別處；`canonical.sfc` 照常產生在實際使用的資料夾。
- 用完或被拒都會刪掉這三個約定檔名（`rom.sfc`、`rom.smc`、`dbz102.ips`），`setup\` 空了才刪資料夾；裡面的其他檔不動，玩家的原檔在別處、不受影響。
- 被拒（大小或版本不對、遊戲讀不了）時，用和現在相同的診斷說明告知一次，然後照舊跳出選檔對話框重選；檔已刪掉，下次不會再拿同一個錯檔報錯。
- 只放了 `dbz102.ips` 沒放 ROM：不用，刪掉。放的是日版 Rev 1 ROM 卻沒有 IPS（啟動器本身也沒有內嵌或附加的 IPS）：只補問 IPS 那一個對話框；按取消就照現行規則告知需要 IPS，再回到選檔對話框。
- 已經有 `canonical.sfc` 時完全不讀 `setup\`，只把留在那裡的約定檔刪掉。自備 ROM 版（exe 檔尾附加 ROM）照舊用附加的 ROM，不看 `setup\`。

- 第一次設定 ROM 成功後，訊息框列出資料夾、存檔與 ROM 複本的路徑，並提醒分享資料夾前先刪 `cache\rom`；exe 旁的 `save\` 不能寫而退回上游路徑時，每次啟動都再提示一次。
- ROM 被拒時，訊息框加上診斷：有 512 bytes 檔頭會自動去掉、大小不對、Klepto 英文版／日版 Rev 1（CRC32 依上游 `identify.rs`）、其他版本需換成 Rev 1；日版 Rev 1 沒有 IPS 時，跑遊戲前就擋下並說明。
