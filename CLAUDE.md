# wimux — 開発ガイド（Claude Code 向け）

Windows 向けの、軽量でワークスペース中心のターミナル。cmux（macOS）のような使い心地を Windows で実現する。

## 目標

- 左サイドバーにワークスペースを並べる。各ワークスペースはタブ・分割ペインを持つ
- ペインごとに **cwd / conda 環境 / Claude Code 起動状態** を記憶し、次回起動時に復元する
- Claude Code など AI エージェントの進捗を、サイドバーのバッジと Windows のトースト通知で知らせる
- ブラウザペイン（WebView2）を持ち、AI からも操作できるようにする
- **起動が速く、メモリを使いすぎない**こと（最優先の非機能要件）

## 技術スタック

| 役割 | 採用 | 備考 |
|---|---|---|
| UI / 描画 | GPUI（`gpui-kit` 0.8 経由。中身は `gpui-pre` 0.3.10） | Zed の UI フレームワーク。GPU 描画・IME 対応済み |
| UI 部品 | 自前（`window.rs`） | タブ・分割は自前の分割ツリーで描画。`gpui-component` は依存に入っているが未使用 |
| 端末エミュレーション | `alacritty_terminal` 0.26 | VT パーサ・グリッド・スクロールバック |
| PTY | `portable-pty` 0.9（Windows では ConPTY） | 実行ファイルの隣に `conpty.dll` + `OpenConsole.exe` を置くとそちらを優先して使う |
| ブラウザペイン（Phase 4） | `wry`（WebView2）を子ウィンドウとして重ねる | GPUI の `Window` は `HasWindowHandle` を実装している |

### なぜ alacritty の `tty` / `EventLoop` ではなく portable-pty か

PTY の出力を **自分で読んでから** alacritty に渡したいため。途中で OSC（cwd・conda・通知）を拾う `OscScanner` を挟んでいる。alacritty の EventLoop を使うとこの差し込みができない。

## ディレクトリ構成

```
wimux/
├─ Cargo.toml                 # ワークスペース
├─ crates/wimux/src/
│  ├─ main.rs                 # アプリ起動・キー割り当て登録
│  ├─ keymap.rs               # コマンドの表（キー割り当て・コマンドパレット・ブラウザから回すキー）と settings.json の keybindings
│  ├─ window.rs               # WimuxWindow: サイドバー・タブバー・分割表示・操作（アクション）・保存復元・通知
│  ├─ workspace.rs            # ワークスペース → 分割ツリー → 区画（タブの並び）のモデル（UI 非依存・テストあり）
│  ├─ session.rs              # session.json の形式と読み書き
│  ├─ dirs.rs                 # フォルダ履歴（cdi 用、zoxide 方式のスコア）
│  ├─ browser.rs              # ブラウザペイン（wry / WebView2 を子ウィンドウとして重ねる）
│  ├─ text_field.rs           # 1 行の入力欄（ブラウザの URL 欄）。IME・貼り付け対応
│  ├─ picker.rs               # ポップアップ（cdi のフォルダ選択・名前入力）。IME 対応
│  ├─ notify.rs               # Windows トースト通知（PowerShell 経由）
│  ├─ ipc.rs                  # ペイン内のフック・`sondev notify` から本体へ知らせる口（127.0.0.1 の TCP ＋トークン）
│  ├─ hooks.rs                # Claude Code のフック / Codex の notify / `sondev notify` → エージェントの状態
│  ├─ claude_session.rs       # Claude Code のフック設定（--settings）と、ペインごとのセッション ID の記録
│  ├─ open.rs                 # Ctrl+クリックで URL・ファイルを外のアプリ（ブラウザ・エディタ）で開く
│  ├─ update.rs               # 新しいバージョンの確認（GitHub Releases）と、［更新］でのダウンロード・上書きインストール
│  ├─ theme.rs                # テーマ（ターミナル配色）と、そこから作る UI 色・文字サイズ・寸法
│  ├─ settings.rs             # settings.json（テーマ・フォント・文字サイズ・Claude 自動再開・エディタ）
│  └─ terminal/
│     ├─ mod.rs               # Terminal: PTY 起動、reader / wait スレッド、リサイズ、問い合わせ応答
│     ├─ view.rs              # TerminalView: 描画・キー入力・IME・選択/コピー・スクロール・貼り付け
│     ├─ keys.rs              # キー → エスケープシーケンス / win32-input-mode
│     ├─ links.rs             # 文字列から URL・ファイルパス（:行:列）を見つける
│     ├─ osc.rs               # OSC スキャナ（cwd / UserVar / 通知）
│     └─ colors.rs            # パレット・色解決
└─ shell-integration/
   └─ wimux.ps1               # PowerShell 連携（cwd・conda 送信、conda 遅延初期化、cdi・clist）※ASCII のみ
```

保存先（`WIMUX_CONFIG_DIR` で変更可）:
- `%APPDATA%\SondevTerminal\session.json` … ワークスペース・分割・区画ごとのタブ・ペインごとの cwd / conda / Claude 起動状態（v2。v1 は読み込み時に変換）
- `%APPDATA%\SondevTerminal\dirs.json` … フォルダ履歴（初回は zoxide の履歴を取り込む）
- `%APPDATA%\SondevTerminal\dirs.txt` … シェルの `cdi` が読むフォルダ一覧（よく行く順。wimux が保存のたびに書き出す）
- `%APPDATA%\SondevTerminal\settings.json` … 設定画面の内容（テーマ・フォント・文字サイズ・行の高さ・カーソル・透過など）

## 配布（インストーラー）

表示名は「Sondev Terminal」、実行ファイルは `sondev.exe`（`crate::APP_NAME` と `crates/wimux/Cargo.toml` の `[[bin]]`）。
コード・フォルダ名・環境変数（`WIMUX_*`）は開発名の wimux のまま。設定の保存先は `%APPDATA%\SondevTerminal`、ブラウザのデータは `%LOCALAPPDATA%\SondevTerminal\WebView2`。以前の `wimux` フォルダは起動時に自動で移す（`session::migrate_dirs`。移せなければ古い場所をそのまま使う）。

```powershell
winget install JRSoftware.InnoSetup               # 初回だけ
cargo install cargo-about --locked --features cli # 初回だけ（ライセンス一覧の生成用・任意）
powershell -ExecutionPolicy Bypass -File installer\build.ps1
```

→ `dist\SondevTerminal-setup-<version>.exe`。バージョンはルートの `Cargo.toml` の `[workspace.package] version`。

**リリース（GitHub Actions）**: Actions → Release → Run workflow でバージョン（例 `0.2.0`）を入れるか、`git tag v0.2.0 && git push origin v0.2.0`。
Windows ランナーでインストーラーを作り、GitHub Releases に置く（バージョンはタグから決めるので Cargo.toml を上げるコミットは不要）。

**winget**（`winget install Sondev.SondevTerminal`）: Release ワークフローが `installer/winget.ps1` で `installer/winget/*.yaml`（ひな形。`{{VERSION}}` などを埋める）から `dist/winget/` にマニフェストを作り、成果物に残す。リポジトリの Secrets に `WINGET_TOKEN`（public_repo 権限の GitHub トークン）があれば、wingetcreate で microsoft/winget-pkgs に PR を出す（プレリリースは出さない）。初回は Microsoft の人の審査がある。ProductCode は Inno Setup の AppId ＋ `_is1`。リポジトリが public で、Release のインストーラーが誰でもダウンロードできることが前提

**CI**: PR と main へのマージでは Linux で fmt・test・`--cfg wimux_check` の型チェックだけ（軽い）。Windows のリリースビルドとインストーラー作成は、Actions → CI → Run workflow で手動実行したときだけ（Windows ランナーは分数が 2 倍で数えられ、1 回 20 分ほどかかるため）。

- `installer/wimux.iss` … Inno Setup の設定。管理者権限なしで `%LOCALAPPDATA%\Programs\Sondev Terminal` に入る。`AppId` は変えない（アップデートの判定に使う）
- `crates/wimux/build.rs` … exe にアイコン（`assets/wimux.ico`、リソース ID 1）とバージョン情報を埋め込む
- シェル連携スクリプトは exe に埋め込み（`include_str!`）、起動時に `%APPDATA%\SondevTerminal\shell-integration\wimux.ps1` へ書き出す
- `about.toml` / `installer/about.hbs` … 使用ライブラリのライセンス一覧（cargo-about）。許可していないライセンスの依存を足すと生成が失敗するので、そのときは中身を確認して `accepted` に追加する
- WebView2 ランタイム・VC++ ランタイムは同梱していない（Windows 10 / 11 には通常入っている）
- 署名していないので、初回起動時に SmartScreen の警告が出る
- **アップデート**: 起動の 5 秒後と 12 時間ごとに `update.rs` が GitHub Releases の最新版を確かめ（`curl.exe`。依存を増やさないため）、新しければサイドバーの下に「新しいバージョン vX ［更新］」。［更新］でインストーラーを %TEMP% にダウンロードし `/SILENT /relaunch=1` で起動、アプリは保存して終了する。インストーラーは上書き後に新しい版を起動する（`wimux.iss` の `RelaunchRequested`）。**リポジトリが public で、Release にインストーラーが付いていること**が前提。開発ビルドでは確認しない（`WIMUX_UPDATE_CHECK=1` で確認、`=0` で常に確認しない。設定画面でも OFF にできる）

## キー操作

| キー | 動作 |
|---|---|
| Ctrl+Shift+P | コマンドパレット（コマンドを名前で検索して実行。今のキーも表示） |
| Ctrl+Shift+N | 新しいワークスペース |
| Ctrl+Shift+PageDown / PageUp | 次 / 前のワークスペース |
| Ctrl+1〜8 / Ctrl+9 | 1〜8 番目 / 最後のワークスペース |
| Ctrl+Shift+R | ワークスペース名の変更（サイドバーのダブルクリック、右クリックメニューでも可） |
| サイドバーで右クリック | 名前を変更 / 新しいワークスペース / 上へ移動 / 下へ移動 / 削除 |
| サイドバーのワークスペースをドラッグ | 並べ替え（落としたワークスペースの位置に入る。一覧の空きに落とすといちばん下） |
| Ctrl+T / Ctrl+Shift+T | フォーカス中の区画に新しいタブ（今のフォルダで開く） |
| Ctrl+] / Ctrl+[（Ctrl+Tab / Ctrl+Shift+Tab） | 区画内の次 / 前のタブ |
| Ctrl+D / Ctrl+Shift+D | 区画を右に分割 / 下に分割（Ctrl+Shift+E も下に分割。各区画のタブ行の ◫ ⊟ でも可） |
| Ctrl+矢印 / Alt+矢印 | 区画間の移動 |
| Alt+Shift+矢印 | 分割の境界をその方向へ動かす（5% ずつ）。境界の線をマウスでドラッグしても変えられる |
| Ctrl+W / Ctrl+Shift+W | タブを閉じる（最後のタブなら区画も閉じる） |
| Ctrl+Shift+L | 右に分割して、新しい区画を空のブラウザにする（URL 欄に入力して Enter）。ブラウザのタブを見ているときは、その URL 欄へ |
| Ctrl+Shift+G / `cdi [キーワード]` | フォルダ履歴からターミナル内で選んで移動（fzf を使う。PATH になければインストーラー同梱の fzf.exe、それもなければ内蔵の選択画面） |
| Ctrl+Shift+A / `clist [キーワード]` | conda の環境をターミナル内で選んで `conda activate`（cdi と同じ選択画面。一覧は conda を起動せず `<root>\envs\*` と `~\.conda\environments.txt` から作る） |
| Ctrl+C | 文字を選択中ならコピー、そうでなければ ^C（中断） |
| Ctrl+V | クリップボードに文字があれば貼り付け。画像だけ（スクショなど）のときは、Claude Code には Alt+V（画像の貼り付け）を送り、それ以外のアプリには Ctrl+V をそのまま送る |
| Ctrl+Shift+C / Ctrl+Shift+V | コピー / 貼り付け |
| 右クリック | 選択があればコピー、なければ貼り付け |
| （アプリがマウスを使うとき） | Claude Code の全画面表示・vim などがマウスを求めているときは、クリック・ドラッグ・右クリックをアプリに送る（ボタンが押せる）。Shift を押しながらなら wimux の文字選択・コピー |
| タブをドラッグ | タブの行で並べ替え・別の区画のタブの行へ移動。区画の上に落とすと、端（25%）なら分割して配置、中央ならその区画のタブに。サイドバーのワークスペースに落とすとそこへ移動、一覧の空きに落とすと新しいワークスペース |
| Ctrl+Enter / Shift+Enter | 改行（Claude Code など） |
| Ctrl+= / Ctrl+- / Ctrl+0 | 文字を大きく / 小さく / 元に戻す |
| Ctrl+Shift+B | サイドバーの表示 / 非表示 |
| Ctrl+, | 設定画面（サイドバー右下の「⚙ 設定」でも可） |
| Ctrl+Shift+O | フォーカス中の区画に空のブラウザのタブを開く（URL 欄に入力して Enter） |
| `web [URL / ポート / 検索語]` | シェルから、同じ区画に新しいブラウザのタブを開く（引数なしなら空のタブ。`web 3000` → localhost:3000） |
| Ctrl+Shift+F | スクロールバックを検索（Enter / ↑ で上へ、Shift+Enter / ↓ で下へ、Esc で閉じる。大文字を含まなければ大文字・小文字を区別しない） |
| Ctrl+クリック | URL を既定のブラウザで、ファイルパス（`src/a.rs:12:3` など）をエディタで、フォルダをエクスプローラーで開く。Ctrl を押すと下線が出る |
| Ctrl+Shift+クリック | URL をアプリ内のブラウザのタブで開く |
| Ctrl+Shift+U | 入力待ち（許可・質問）→ 完了して未確認 → そのほかの通知 の順に、次のペインへ移動（押すたびに巡回） |
| `sondev notify [--title 題] 本文` | シェルから通知を出す（`cargo build; sondev notify ビルド完了` など） |

キー割り当ては `keymap.rs` の `COMMANDS`（コマンド id・表示名・既定のキー）。ターミナル（`TerminalView::on_key_down`）より先にキー割り当てが処理される。
settings.json の `keybindings` で変えられる（キー → コマンド id。空文字列なら既定の割り当てを外す。再起動後に反映）:

```json
"keybindings": { "ctrl-w": "", "ctrl-alt-w": "close_tab" }
```

ブラウザ（WebView2）にフォーカスがあるときは、wimux のキー（Ctrl / Alt 付きのもの）を WebView2 の AcceleratorKeyPressed で拾って wimux に回す（`browser.rs` の `forward_keys` → `BrowserMsg::Key`）。Ctrl+C / Ctrl+V / Ctrl+F など、ブラウザで使うキーは回さない（`keymap.rs` の `BROWSER_KEYS`）。
そのため Ctrl+D（EOF）・Ctrl+[（Esc と同じ）・Ctrl+←→（PSReadLine の単語移動）・Ctrl+T・Ctrl+W（単語の削除）はシェルやアプリ（Claude Code の Ctrl+T など）には届かない。

## 環境変数

| 変数 | 意味 |
|---|---|
| `WIMUX_CONFIG_DIR` | 保存先フォルダ |
| `WIMUX_RESTORE_CLAUDE=0` | 復元時に `claude --continue` を実行しない |
| `WIMUX_WIN32_INPUT=0` | win32-input-mode を使わない（キー入力の不具合切り分け用） |
| `WIMUX_PTY_LOG=<path>` | PTY の生出力をファイルに記録（デバッグ用） |
| `WIMUX_CONDA_ROOT` | conda の場所（自動検出できないとき） |
| `WIMUX_UPDATE_CHECK=0` / `=1` | 新しいバージョンを確認しない / 開発ビルドでも確認する |
| `WIMUX_IPC` / `WIMUX_EXE` | （wimux がペインに渡す）フック・`sondev notify` の送り先と、sondev.exe の場所 |

## ビルド・実行（Windows）

前提:
- Rust（rustup、`x86_64-pc-windows-msvc`）
- Visual Studio Build Tools（「C++ によるデスクトップ開発」＋ Windows 11 SDK）

```powershell
cargo run                     # 開発ビルドで起動（コンソールにログが出る）
cargo run --release           # 速度・メモリの確認はこちらで
cargo test                    # OSC スキャナ・キー変換のテスト
$env:RUST_LOG="info"; cargo run   # ログを詳しく
```

初回ビルドは GPUI のコンパイルで数分かかる。

## 構成（cmux と同じ）

ワークスペース → 分割ツリー（`Node`）→ 区画（`Group`: タブの並び）→ タブ（= ターミナル 1 つ、`Pane`）。
分割するとその区画の横 / 下に新しい区画ができ、区画ごとに自分のタブ行を持つ。

## 見た目（cmux / herdr を参考）

- テーマは `theme.rs` の `THEMES`（ターミナルの 16 色＋前景・背景だけを定義）。UI の色は `derive()` で自動生成し、コードからは `th().text` のように参照する。テーマを足すときは `THEMES` に 1 件追加するだけでよい
- 背景の透過: 設定の `opacity`（1.0 = 透過なし）と `backdrop`（透かす / ぼかし / Mica / Mica Alt。既定は透かす）。後ろのウィンドウが見えるのは「透かす」だけ（ぼかしは Windows のアクリル効果で白っぽくぼやけ、Mica は壁紙の色だけが出る）。settings.json の `version` が 0 の「ぼかし」は読み込み時に「透かす」に直す。`window.rs` の `apply_backdrop` で `WindowBackgroundAppearance` を切り替え、背景は `theme::bg()` で透明度つきで塗る。重ね塗りで濃くならないよう、透過中はルートとターミナル本体（`view.rs`）は背景を塗らず、サイドバー・タブの行・区画の枠だけが塗る。gpui-kit の Root（gpui-component）はいちばん下にテーマの背景（既定のライトテーマでは白）を塗るので、`main.rs` で Root の背景を透明にしている（これが無いと透過が白く濁るだけになる）
- 行の高さ（`theme::line_height()`）とカーソルの形（`terminal::set_default_cursor` → alacritty の `default_cursor_style`）も設定画面から変更できる
- 今のターミナル配色は `terminal::colors::current()`（PTY の読み取りスレッドからも参照するためグローバル）
- 文字サイズ: ターミナル 13（Ghostty / cmux の既定）、UI 13 / タブ 12.5 / 補足 11.5
- プロンプト: 標準の `PS パス>` のときは git のブランチ名を `PS C:\repo (main)>` のように表示（.git/HEAD を直接読むので速い。`$env:WIMUX_GIT_PROMPT = "0"` で無効）
- エージェントの状態の丸（タブ・サイドバー）: 黄=実行中、赤=入力待ち（許可・質問で停止）、緑=完了して未確認、灰=確認済みで待機、青=その他の通知。見ていない区画は同じ色の枠で囲み、未読数のバッジも同じ色
- エージェントの状態の取り方（`window.rs` の `AgentState`）:
  - Claude Code: シェル連携の `claude` 関数から起動すると、フック（SessionStart / UserPromptSubmit / PostToolUse / Notification / Stop / SessionEnd）が `sondev.exe --claude-hook` を呼び、`ipc.rs` 経由で本体に届く。フックが無いとき（直接 claude.exe を実行など）はタイトル・ベル・OSC 通知から推定
  - Codex / Gemini / Aider / OpenCode / Copilot など: シェル連携のラッパー関数が起動時に `WIMUX_AGENT` を送り、出力の動き（2 秒止まったら待機中）で推定。ベル・OSC 9 / 777 の通知は入力待ち。Codex は `-c notify=[sondev.exe, --codex-notify]` を付けて起動し、応答の完了を正確に受け取る（config.toml に notify があればそちらを優先）
  - 入力待ちのあとにキーを押したら「作業中」に戻す
- Claude の利用上限（5 時間枠・週）: フック設定（`--settings`）に `statusLine`（`sondev.exe --claude-statusline`）を入れ、ステータスラインの入力の `rate_limits.five_hour` / `seven_day`（`used_percentage`・`resets_at`）を `ipc.rs` で本体へ送る。サイドバーの下に棒で表示（70% 以上は黄、90% 以上は赤）し、Claude の画面の下にも「モデル · 5h 23%（あと 2時間13分） · 週 41%」を出す。Pro / Max などのサブスクリプションで、最初の応答のあとにだけ届く。ユーザーが ~/.claude/settings.json に自分の statusLine を設定しているときは入れない（`--settings` が優先されて消してしまうため）
- ファイルを開くエディタは settings.json の `editor`（例: `"code -g {file}:{line}:{col}"`）。未設定なら VS Code → Cursor → Windsurf → 既定のアプリ
- Claude Code の再開: シェル連携の `claude` 関数が `--settings claude-hook-settings.json` を付けて起動し、SessionStart フック（`sondev.exe --claude-hook`）がペインごとのセッション ID を `claude-sessions\<WIMUX_PANE_ID>` に書く。保存時に読んで session.json に記録し、次回は `claude --resume <ID>`（ID が無ければ `--continue`）
- Codex などの再開: Codex の notify（`--codex-notify`）の `thread-id` を IPC でペインに送り、session.json の `agent` / `agent_session` に保存。次回は `codex resume <ID>`（無ければ `codex resume --last`。cwd で絞られる）。OpenCode / Copilot は `--continue`、Gemini は `--resume latest`（`window.rs` の `agent_resume_command`。方法が分からないエージェントは再開しない）
- Codex の利用上限: notify のたびに `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-*-<thread-id>.jsonl` の末尾から最後の `token_count` の `rate_limits`（primary / secondary の `used_percent`・`window_minutes`・`resets_at`）を読み、`Message::Usage { agent: "codex" }` で送る（`hooks.rs` の `codex_usage`）。枠の長さが 1 日以下なら 5 時間枠、それより長ければ週
- conda の再開: base も含めて `conda activate` する（conda は遅延初期化なので base も自動では有効にならない）
- URL 欄（text_field.rs）: カーソル移動・選択・コピー/切り取り/貼り付け・マウス選択に対応。GPUI の examples/input.rs と同じく自前で描画
- ウィンドウ: 閉じたときの位置・大きさ・最大化を `window.json` に保存し、次の起動で復元（初回や、前回の位置がどの画面にも入らないときは最大化）
- サイドバー: ワークスペースごとに「状態の丸・名前・未読数」「ブランチ · フォルダ」「conda · Claude の状態」「最新の通知」
- 状態の丸: 黄=Claude 作業中、青=未確認の通知あり、緑=Claude 入力待ち
- 見ていない間に通知があったペインは青い枠（cmux の pane ring）。フォーカスすると消える
- Windows では OS のタイトルバーを消し、区画のタブ行を一番上に置く。上端の区画のタブ行の空きはウィンドウのドラッグ（`WindowControlArea::Drag`）、右上の区画のタブ行の右端に最小化・最大化・閉じるボタン（`custom_titlebar()` / `Edge`）。Linux で確認するときは `WIMUX_CUSTOM_TITLEBAR=1`

## 設計ルール

- **UI スレッドをブロックしない**。PTY の読み取り・パースは reader スレッドで行い、UI には `TermMsg` で知らせる
- 再描画はまとめる（`view.rs` の I/O タスクで 8ms 間隔に間引いている）
- ペインごとに環境変数 `WIMUX=1` と `WIMUX_PANE_ID` を渡す。外部（Claude Code の hooks など）からペインを特定するのに使う
- 普通の文字入力は `on_key_down` で処理せず IME（`EntityInputHandler`）に任せる。英字も日本語も同じ経路で確定文字列が届く
- フォントは必ず等幅。`view.rs` の `FONT_CANDIDATES` の先頭から、インストール済みの最初のものを使う
- セルの描画は「同じスタイルが続く範囲」をまとめて shape し、`force_width` でセル幅に固定する。全角は 1 文字ずつ幅 2 セルで描く
- コメント・UI 文言は日本語でよい
- 依存を増やすときは、起動時間とバイナリサイズへの影響を考える

## フェーズ

### Phase 0: 技術検証 ✅
Windows で確認済み: PowerShell・色・全角・cwd/conda 表示（OSC 9;9 は ConPTY を通る）・Claude Code（全画面表示）・Ctrl+Enter 改行。
- [ ] 日本語 IME の確認
- [ ] `--release` での起動時間とメモリの記録

### Phase 1: 骨格 ✅（Linux で動作確認済み、Windows 未確認）
- [x] ワークスペース → タブ → 分割ツリーのモデル（`workspace.rs`）
- [x] サイドバー: 追加・切り替え・名前変更（ダブルクリック）・削除（ホバーで ✕）
- [x] タブバー・分割（左右 / 上下）・Alt+矢印でペイン移動
- [x] 表示されるまでシェルを起動しない（遅延起動）
- [x] 文字選択とコピー（ドラッグ、ダブルクリックで単語、トリプルクリックで行）
- [x] スクロールバック検索（Ctrl+Shift+F）
- [x] URL・ファイルパスの Ctrl+クリック（OSC 8 のリンクにも対応）
- [x] 分割境界のドラッグでサイズ変更（`DraggedDivider`。比率は session.json に保存）・Alt+Shift+矢印
- [x] キー割り当ての変更（settings.json の `keybindings`）・コマンドパレット（Ctrl+Shift+P）
- [ ] 設定ファイル（シェルの選択など）
- [x] タブのドラッグ並べ替え、ワークスペースの並べ替え（サイドバーでドラッグ・右クリックの上へ / 下へ移動）

### Phase 2: 記憶と復元 ✅
- [x] レイアウト＋ペインごとの cwd / conda / Claude 起動状態を保存（変更から 0.8 秒後・終了時）
- [x] 復元: その cwd でシェル起動 → プロンプトが出るたびに `conda activate` → `claude --continue` を順に送る
- [x] cdi: フォルダ履歴（zoxide 方式）＋ポップアップで選択、zoxide 履歴の取り込み

### Phase 3: AI 連携（一部済み）
- [x] Claude Code の状態をタイトルから推定（`✳ Claude Code`=入力待ち、先頭がスピナー=実行中）。サイドバーとタブに表示
- [x] 実行中 → 入力待ちになったら通知（見えていないワークスペースは未読バッジ、ウィンドウが裏ならトースト）
- [x] OSC 9 / 777 の通知も同じ扱い（Claude Code の `preferredNotifChannel` を `iterm2` などにすると届く）
- [ ] トースト通知の Windows での動作確認
- [x] `sondev notify` CLI ＋ Claude Code hooks 連携（127.0.0.1 の TCP ＋トークン。hooks の出力は端末に出ないため）
- [x] Claude 以外のエージェント（Codex・Gemini・Aider など）の状態表示と完了通知
- [x] 入力待ちのペインへ移動（Ctrl+Shift+U）
- [x] Claude の利用上限（5 時間枠・週）の表示（ステータスライン経由）・Codex の利用上限（rollout の記録から）
- [x] Codex・OpenCode・Copilot・Gemini の会話の再開
- [ ] git worktree 連携（ブランチごとのワークスペース作成・差分・PR・片付け）
- [ ] ワークスペース単位で Claude 自動再開の ON/OFF

### Phase 4: ブラウザペイン（実装済み・Windows 未確認）
- [x] `wry` で WebView2 を、メインウィンドウに所有された枠なしウィンドウに入れて区画の位置に重ねる（`browser.rs`）。タブの 1 種類として扱い、URL は保存・復元する
- [x] 操作バー（戻る・進む・再読み込み・URL 欄）。URL 欄をクリックすると入力ポップアップ
- [x] 新しいウィンドウの要求（右クリックの「新しいウィンドウで開く」・`target="_blank"`・`window.open`）は別ウィンドウにせず、同じ区画の新しいタブで開く（`with_new_window_req_handler` → `BrowserMsg::NewWindow`）
- [x] 見えない区画・別ワークスペース・ポップアップ表示中は `set_visible(false)` で隠す（子ウィンドウは GPUI の上に重なるため）
- [x] 設定で「AI からブラウザを操作」を ON にすると `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222`（再起動後に反映）
- [x] そのとき `%APPDATA%\SondevTerminal\claude-mcp.json`（Playwright MCP を `--cdp-endpoint http://127.0.0.1:9222` でつなぐ設定）を書き出し、シェル連携の `claude` 関数が `--mcp-config` で渡す（`claude_session::write_mcp_config`）。`claude mcp add` は不要で、どのプロジェクトでも MCP「sondev-browser」が使える。作業記録は `--output-dir %APPDATA%\SondevTerminal\playwright-mcp` に出す（プロジェクトに `.playwright-mcp/` を作らない）。Chrome を別に起動しないこと（9222 は Sondev の WebView2 が使っている）
- [ ] Windows での動作確認（Linux はブラウザペイン非対応）
- [x] ブラウザをクリックしたら、その区画・タブをアクティブにする（WebView2 の GotFocus → `BrowserMsg::Focused`。クリックは別ウィンドウに行くので GPUI には届かない）
- [x] ブラウザにフォーカスがある間も wimux のキー操作を効かせる（AcceleratorKeyPressed で拾って転送）

## 既知の注意点

- ConPTY は起動直後にカーソル位置の問い合わせ（`ESC[6n`）を送ってくる。`EventProxy` が `PtyWrite` を PTY に書き戻して応答している。ここを壊すとシェルが固まる
- `shell-integration/wimux.ps1` は **ASCII のみ**で書く。Windows PowerShell 5.1 は BOM なしファイルを Shift_JIS として読むため、日本語コメントがあると構文が壊れてプロンプトが `PS>` になる
- conda はシェル連携スクリプトで「初回の `conda` 実行時にだけ初期化する」スタブを定義している（起動を遅くしないため）
- TUI（Claude Code など）は背景色 (OSC 11) や画面サイズ (CSI 14 t) を問い合わせてくる。`EventProxy` で応答している
- ConPTY は子プロセス終了後も read が EOF にならないことがあるため、終了検知は wait スレッドで行っている
- GPUI（`gpui-pre`）は API の変更が多い。バージョンを上げるときは `gpui-kit` と揃えて上げる
- 参考実装: Zed の `crates/terminal` / `crates/terminal_view`（GPL なのでコードはコピーしない。読むだけ）、`gpui-terminal` クレート（MIT/Apache）
- `use gpui_kit::*` すると GPUI の `test` マクロが Rust 標準の `#[test]` を隠してしまう。テストモジュールでは `use super::*` を避け、必要なものだけ import する
- gpui-kit の Root は `tab` / `shift-tab`（フォーカス移動）と `ctrl-c`（コピー）にキーを割り当てている。キー割り当てはターミナルの `on_key_down` より先に処理されるため、`window.rs` で `Terminal` コンテキストに `NoAction` を割り当てて無効にしている。ターミナルで効かないキーがあれば、まずここを疑う
- ブラウザ（別ウィンドウ）を隠す条件は `sync_browsers` と `render_browser` で必ず揃える。ずれると描画のたびに「隠す → 表示」を繰り返してチカチカする。境界のドラッグ（`resizing`）中は隠さず、タブ・ワークスペースのドラッグ中だけ隠す（`tab_dragging`）
- WebView2 の作成中は Windows のメッセージループが回る。GPUI の処理の途中（render や action の中）で作ると GPUI が二重に呼ばれて壊れるので、`sync_browsers` では生のウィンドウハンドルだけを持って非同期タスクの中で作っている
- GPUI（Windows）は DirectComposition の描画を子ウィンドウより **上** に重ねる（`CreateTargetForHwnd(hwnd, true)`）ため、WebView2 を子ウィンドウにすると隠れて見えない。そこで `browser.rs` の `host::HostWindow`（メインウィンドウに所有された枠なしウィンドウ）に WebView2 を入れ、区画の位置（画面座標）に重ねている。メインウィンドウを動かしたときは window.rs の 30ms タイマーで `reposition` して追従させる
- Windows 専用コードは Linux でも型チェックできる: `RUSTFLAGS="--cfg wimux_check" cargo check`。`rustup target add x86_64-pc-windows-msvc` すれば `cargo check --target x86_64-pc-windows-msvc` で Windows 向けそのものも型チェックできる（リンクはしない）
