# assets/：build 前放進來、會被嵌進啟動器 exe 的檔案

這個目錄除了本檔與 `.gitignore` 都不進 git（大檔、二進位、不可散布物）。用 `../fetch_assets.sh <來源資料夾>`
複製進來。`ssd-recompiled.exe` 由本分支原始碼編出（見 `../BUILD.zh-TW.md` 第 1 步），台版譯文與字型取自本分支的 `translations/zh-tw/`、港版譯文取自 `translations/zh-hk/`；
先依下表的「來源資料夾內的位置」排好，再執行 fetch_assets.sh。`build.rs` 會把這裡的每個檔案嵌進 exe，解開時保留相對路徑。

| 檔案 | 來源資料夾內的位置 | 必要 |
|---|---|---|
| `ssd-recompiled.exe` | `win/dbz-ssd-recompiled-windows-v2/`（本分支 `cargo build --release -p ssd-app` 編出的 `target\release\ssd-recompiled.exe`；修補都在本分支的 commit 裡，不另外套 patch） | 是 |
| `symbols.toml` | 同上（只有面板切換 hook 時用得到；預設 `--hooks original` 不讀） | 否 |
| `LICENSE.txt`、`NOTICE.md`、`PATCH_NOTES.txt`、`licenses/*` | 同上（GPL-3／OFL 授權，隨程式附上） | 建議 |
| `zh-tw.tsv`、`ui-zh-tw.tsv` | 根目錄（取自本分支的 `translations/zh-tw/`） | 是 |
| `zh-hk.tsv`、`ui-zh-hk.tsv` | 根目錄（取自本分支的 `translations/zh-hk/`；啟動器每次寫進資料目錄，`build.rs` 檢查） | 是 |
| `fonts/LXGWWenKaiTC-Regular.ttf`、`fonts/OFL.txt` | `fonts/`（取自本分支的 `translations/zh-tw/fonts/`，霞鶩文楷 TC 1.530） | 是（ttf） |
| `art-override/<art id>.png`、`art-override/super-vegeta/<art id>.png` | `art-override/`（super-vegeta/ 是超級達爾專用圖，檔名用普通達爾的圖名；改作自 HD 美術包原圖的圖（如超級達爾去墊肩）：Ming 以 Google Gemini 圖像模型編輯自 HD 美術包原圖，CC BY-NC-SA 4.0，`gallery-party-piccolo-u01-061.png`、`variants/battle-goku-ssj-020.png`、`super-vegeta/key-*.png` 與 `ssj-lightning/` 除外，見 `LICENSE-ART.txt`；每張與美術包原圖同尺寸、RGBA。啟動器第一次執行時寫進資料目錄的 `art-override\`，遊戲用它取代美術包同名的圖） | 是（至少一張） |
| `art-override/super-vegeta/key-<鍵值>.png`、`art-override/super-vegeta/key-<鍵值>.pose` | `art-override/super-vegeta/`（美術包完全沒有美術的超級達爾姿勢，遊戲 `ssd_hd::pack::KEYED_PICTURE`，例如悟空龜派氣功命中時的背面、側身被推與翻滾：`<鍵值>` 是姿勢的 16 位十六進位鍵值，大小寫皆可。發行版附的是 `.pose`：一行 `key=<鍵值> size=<寬>x<高> colours=<用到的色號> mirror=<左右鏡像的鍵值>`，只記姿勢的鍵值、大小、用到的色號與鏡像鍵值，不含原版像素，由 `cargo run -p xtask -- keyed-poses <資料夾>` 從開發端的 `.txt` 產生；遊戲沒有 `.txt` 時讀它（`ssd_hd::pack::KEYED_POSE_EXTENSION`）。開發端的 `.txt` 是同一姿勢的原版像素（只留在來源夾，fetch_assets.sh 不複製，啟動器不寫，build.rs 看到 assets/ 裡有就拒絕建置）：每行是姿勢框的一列、由上到下，每格一個十六進位數字〔調色盤索引，`0` 透明〕，每行一樣長，空行與行尾空白略過；遊戲由這些像素算出鍵值，須等於檔名的鍵值。PNG 須是姿勢框的 4 倍大小。遊戲把它當成新增的姿勢〔id `key-<鍵值>@super-vegeta`〕，只在超級達爾的配色時使用，普通達爾與其他配色照舊畫原版像素。沒有 `.txt` 也沒有 `.pose`、像素或 `.pose` 的鍵值與檔名不符、尺寸不對、讀不到、美術包已有這個鍵值〔含左右鏡像的鍵值〕，或超級達爾的美術沒有它用到的顏色時，這張圖不用，log 寫原因。fetch_assets.sh 只複製檔名完全是 `key-<16 位十六進位>.pose` 的檔，PNG 沒有同名 `.pose`（或反過來）就停下；啟動器寫進資料目錄的 `art-override\super-vegeta\`） | 否 |
| `art-override/ssj-lightning/frame-<N>.png` | `art-override/ssj-lightning/`（悟空第一次變身超級賽亞人時疊在畫面上的閃電，遊戲 `ssd_hd::lightning::DIR`；檔名 `frame-1.png`～`frame-4.png`，最多 4 張，變身期間每 4 幀換一張輪播；做成 1024×896 的黑底 RGB 或 RGBA，其他尺寸會等比縮放到畫面。啟動器寫進資料目錄的 `art-override\ssj-lightning\`；刪掉這個資料夾即關閉閃電，啟動器不會再寫回來） | 否 |
| `art-override/mode7/<content key>/t<x>_<y>.png`、`palette.txt`、`replace` | `art-override/mode7/<content key>/`（兩層：每個 Mode 7 平面一個資料夾，名稱是 16 位十六進位的內容鍵〔遊戲 `ssd_hd::pack::MODE7_DIR`〕；只複製 PNG 圖塊、可選的 `palette.txt` 與可選的空檔 `replace`，檔名須完全相同，其他檔不複製。啟動器寫進資料目錄的 `art-override\mode7\<content key>\`） | 否 |
| `dbz102.ips` | 根目錄；**只有 `--features bundle-ips` 才會嵌入**，否則就算放在這裡也會被略過 | 否 |
| `voice/ssj-transform.wav` | `voice/`（悟空第一次變身超級賽亞人的日文語音，語音為 AI 生成，非原作錄音；啟動器把它裝進資料夾的 `voice\`，那裡已有不同的檔就不動） | 是 |

**ROM（`.sfc`／`.smc`）放進這裡會讓 build 直接失敗**：ROM 絕不嵌入程式本體。
