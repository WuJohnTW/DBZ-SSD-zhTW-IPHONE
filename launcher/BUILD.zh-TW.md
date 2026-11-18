# 從本分支建出 release 的啟動器 exe

release 附件裡的 `DBZ_超賽亞傳說_繁中_RC1_無ROM.exe` 就是這個資料夾的 crate（`dbz-zhtw-launcher`，GPL-3.0-or-later）建出來的。它把遊戲本體 `ssd-recompiled.exe`、語言包、字型與授權檔嵌進一個 exe。功能與執行流程見同資料夾的 `README.md`。

`assets/README.md` 寫的是用 `fetch_assets.sh` 從整理好的來源資料夾複製；不用來源資料夾時，每個要嵌入的檔在本分支的位置見下表。

## 1. 建遊戲本體

在 repo 根目錄：

```powershell
. tools\no-local-paths.ps1
cargo build --release -p ssd-app
```

`tools\no-local-paths.ps1` 讓 exe 裡不留建置機器的資料夾路徑（使用者資料夾、cargo 的資料夾、repo 的位置）；檢查方式寫在該檔開頭。macOS／Linux 用 `. tools/no-local-paths.sh`。

產出 `target\release\ssd-recompiled.exe`，就是上游 v0.3.15 加上本分支修補的遊戲本體。

## 2. 把要嵌入的檔放進 `launcher\assets\`

| `launcher\assets\` 裡的檔 | 從本分支哪裡複製 | 必要 |
|---|---|---|
| `ssd-recompiled.exe` | `target\release\ssd-recompiled.exe`（第 1 步） | 是 |
| `zh-tw.tsv`、`ui-zh-tw.tsv` | `translations\zh-tw\` | 是 |
| `zh-hk.tsv`、`ui-zh-hk.tsv` | `translations\zh-hk\` | 是 |
| `fonts\LXGWWenKaiTC-Regular.ttf`、`fonts\OFL.txt` | `translations\zh-tw\fonts\` | 是（ttf） |
| `LICENSE.txt` | `LICENSE`（改名） | 建議 |
| `NOTICE.md` | `NOTICE.md` | 建議 |
| `licenses\*` | `translations\zh-tw\licenses\` | 建議 |
| `symbols.toml` | `re\symbols.toml` | 否 |
| `PATCH_NOTES.txt` | 版本說明，不在本分支裡；沒有也能建 | 否 |
| `art-override\*.png`、`art-override\super-vegeta\*.png` | 超級達爾去墊肩成品（super-vegeta\ 是超級達爾專用圖；改作自 HD 美術包原圖的圖由 Ming 以 Google Gemini 圖像模型編輯，CC BY-NC-SA 4.0，`gallery-party-piccolo-u01-061.png`、`variants\battle-goku-ssj-020.png`、`super-vegeta\key-*.png` 與 `ssj-lightning\` 除外，見 `LICENSE-ART.txt`；與美術包原圖同尺寸、RGBA）。定稿前不在本分支裡，由打包的人放入 | 是（至少一張，`build.rs` 檢查） |

在 repo 根目錄用 PowerShell：

```powershell
$a = 'launcher\assets'
New-Item -ItemType Directory -Force "$a\fonts", "$a\licenses" | Out-Null
Copy-Item target\release\ssd-recompiled.exe, translations\zh-tw\zh-tw.tsv, translations\zh-tw\ui-zh-tw.tsv, translations\zh-hk\zh-hk.tsv, translations\zh-hk\ui-zh-hk.tsv, NOTICE.md, re\symbols.toml $a
Copy-Item LICENSE "$a\LICENSE.txt"
Copy-Item translations\zh-tw\fonts\* "$a\fonts\"
Copy-Item translations\zh-tw\licenses\* "$a\licenses\"
```

`assets\` 裡的檔不進 git（`assets\.gitignore`）。

## 3. 建啟動器

需要 Rust（`launcher\rust-toolchain.toml` 指定 1.99）和 Visual Studio 的 C++ build tools（版本資訊資源要用 `rc.exe`）。在 `launcher\` 裡：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File build_windows.ps1 build
```

`build_windows.ps1` 會先載入 `..\tools\no-local-paths.ps1`。也可以載入 vcvars64 與 `..\tools\no-local-paths.ps1` 後直接執行 `cargo build --release`。這個 crate 有自己的 `[workspace]`，不屬於根目錄的 workspace。

產出 `launcher\target\release\dbz-zhtw-launcher.exe`，改名成 `DBZ_超賽亞傳說_繁中_RC1_無ROM.exe` 就是 release 的那個檔。用 `--selftest` 可以檢查嵌入的檔是否完整。

## ROM 與補丁檔

- 本 release 的 exe **不含 ROM，也不含 `dbz102.ips`**。
- `build.rs` 發現 `assets\` 裡有 `.sfc`／`.smc` 時，build 直接失敗。
- feature `bundle-ips` **預設關閉**：只有加 `--features bundle-ips` 時才會嵌入 `assets\dbz102.ips`。本 release 沒有開這個 feature。
- 啟動器會讀 exe 檔尾的附加資料（格式見 `src/overlay.rs`）；本 release 的 exe 沒有附加資料。

## 致謝

- 原作：[JackRushante/DBZ-Super-Saiya-Densetsu-Rust](https://github.com/JackRushante/DBZ-Super-Saiya-Densetsu-Rust)（GPL-3.0）。本分支基於其 v0.3.15（`b4a67c95`），啟動器裡的遊戲本體由本分支原始碼編出，對應的原始碼就是本分支。
- HD 美術包：上游專案 JackRushante/DBZ-Super-Saiya-Densetsu-Rust 的作品，授權 CC BY-NC-SA 4.0（上游 `LICENSE-ART.md`）。美術包不嵌入啟動器、不隨 release 附上，由玩家自備。
- 字型：霞鶩文楷 TC（LXGW WenKai TC 1.530，SIL Open Font License 1.1，`translations\zh-tw\fonts\OFL.txt`）。
