# Sondev Terminal

**AI エージェントと並んで作業するための、Windows 向けターミナル。**

ワークスペースを左に並べて、プロジェクトごとにタブ・分割・ブラウザをまとめておけます。
Claude Code などのエージェントが「作業中」なのか「あなたの返事を待っている」のかがひと目で分かり、
閉じて開き直しても、フォルダ・conda 環境・Claude の会話がそのまま戻ります。

GPU で描画するネイティブアプリ（GPUI ＋ alacritty_terminal ＋ ConPTY）なので、軽くて速いのが特徴です。

---

## 特徴

### ワークスペースで作業を分ける
- 左のサイドバーにワークスペースを並べ、プロジェクトごとに切り替えます（Ctrl+1〜9）
- 各ワークスペースの中で、タブと分割（左右・上下）を自由に組み合わせられます
- サイドバーには、ワークスペースごとに **git のブランチ・フォルダ・conda 環境・エージェントの状態・最新の通知** が出ます
- タブはドラッグで並べ替え・別の区画へ移動・分割して配置。ワークスペースもドラッグで並べ替えられます
- **右側のファイル一覧**（Ctrl+Shift+X）: 今いるフォルダのファイルをツリーで表示します。`cd` すると追いかけ、フォルダはクリックで開く / 閉じる、ファイルはクリックで **Sondev の中のタブで表示**（Ctrl+クリックならエディタで開く）。git のリポジトリの中なら、変更したファイルは黄、新しいファイルは緑、衝突は赤、.gitignore で無視されているものは薄く表示します（変更を含むフォルダにも色が付きます）
- **ファイルの表示**: 行番号とシンタックスハイライト付きで読めます（Rust・Python・JavaScript / TypeScript・Go・C / C++・C#・Java・HTML / CSS・JSON・YAML・TOML・Markdown・SQL・シェルなど。色はテーマに合わせます）。ドラッグで選択して Ctrl+C でコピー、Ctrl+A ですべて選択、トリプルクリックで 1 行選択、Shift+ホイールで横スクロール。上の［エディタで開く］でエディタへ、［↻ 読み直す］で最新に。表示だけで書き換えはできません。開いていたファイルのタブは次の起動でも戻ります

### AI エージェントの状態が分かる
- **Claude Code**・Codex・Gemini・Aider・OpenCode・Copilot などに対応
- タブとサイドバーの丸の色で状態が分かります

  | 色 | 意味 |
  |---|---|
  | 🟡 黄 | 作業中 |
  | 🔴 赤 | 入力待ち（許可・質問で止まっている） |
  | 🟢 緑 | 終わった（まだ見ていない） |
  | ⚪ 灰 | 待機中（確認済み） |
  | 🔵 青 | そのほかの通知 |

- 見ていないワークスペースで何か起きると未読バッジが付き、ウィンドウが裏にあれば Windows の通知も出ます
- **Ctrl+Shift+U** で「入力待ち → 終わった → 通知」の順に、次に見るべきペインへ飛べます
- Claude Code はフックで状態を正確に受け取ります（シェルの `claude` から起動するだけで有効）
- **Claude の利用上限（5 時間枠・週）** をサイドバーの下に棒で表示します（何 % 使ったか・あと何時間で戻るか。70% で黄、90% で赤）。Pro / Max などのサブスクリプションで、Sondev から `claude` を起動して最初の応答があると出ます
  - ご自身で Claude Code のステータスラインを設定している場合は、それを消さないように表示しません
- **Codex の利用上限（5 時間枠・週）** も同じように出ます（Sondev から `codex` を起動すると、Codex が開いている間 20 秒ごとと応答が終わるたびに、Codex の記録 `~/.codex/sessions` から読みます。最初の応答のあとに出ます）

### 閉じても、そのまま続きから
- ワークスペース・分割・タブ、各ペインのフォルダ・conda 環境を自動で保存し、次の起動で元に戻します
- Claude Code を使っていたペインは、**そのペインの会話を `claude --resume` で再開**します
- Docker のコンテナに入っていたペイン（`dlist`・`docker exec -it`・`docker attach`）は、そのコンテナに入り直します（止まっていれば起動）
- Codex は `codex resume <ID>`（ID が分からなければ `codex resume --last`）、OpenCode / Copilot は `--continue`、Gemini は `--resume latest` で再開します
- ウィンドウの位置・大きさも覚えています

### ブラウザも同じ画面で
- 区画の 1 つをブラウザにできます（WebView2）。**Ctrl+Shift+L** で右に分割してブラウザ、`web 3000` で localhost:3000
- 設定で「AI からブラウザを操作」を ON にすると、**Sondev の中で起動した Claude Code がそのブラウザを操作できます**（設定不要。下の「Claude にブラウザを操作してもらう」を参照）
- ブラウザを触っている間も、Sondev のショートカットはそのまま効きます

### ターミナルとしての基本
- スクロールバック検索（Ctrl+Shift+F）
- URL・ファイルパスを **Ctrl+クリック** で開く（`src/main.rs:42:7` ならエディタでその行へ）
- 日本語入力（IME）、全角文字、24bit カラー
- フォルダ履歴からの移動（`cdi`、fzf 対応）
- conda の環境を一覧から選んで有効にする（`clist`。`conda info -e` → `conda activate` の代わり）
- Docker のコンテナを一覧から選んで入る（`dlist`）。サイドバーに 🐳 コンテナ名
- テーマ・フォント・文字サイズ・行の高さ・カーソル・背景の透過を設定画面から変更
- コマンドパレット（Ctrl+Shift+P）で、すべての操作を名前で探して実行
- キー割り当ては settings.json で変更できます
- 新しいバージョンが出ると、サイドバーに［更新］が出てワンクリックで更新できます

---

## インストール

### winget で入れる（おすすめ）
PowerShell（またはコマンドプロンプト）で:

```powershell
winget install Sondev.SondevTerminal
```

| やりたいこと | コマンド |
|---|---|
| 入れる | `winget install Sondev.SondevTerminal` |
| 新しい版にする | `winget upgrade Sondev.SondevTerminal`（アプリ内の［更新］ボタンでも更新できます） |
| 入っている版を見る | `winget list Sondev.SondevTerminal` |
| 消す | `winget uninstall Sondev.SondevTerminal`（設定・ワークスペースは `%APPDATA%\SondevTerminal` に残ります） |

- 管理者権限は要りません（ユーザーごとに `%LOCALAPPDATA%\Programs\Sondev Terminal` に入ります）
- 「見つかりません」と出るときは、`winget source update` で一覧を新しくしてから、もう一度試してください
- 新しい版は、リリースしてから winget に反映されるまで数時間〜数日かかることがあります。すぐに使いたいときは、下のインストーラーかアプリ内の［更新］を使ってください
- `winget` 自体が無いときは、Microsoft Store の「アプリ インストーラー」を入れると使えるようになります（Windows 11 には最初から入っています）

### インストーラーで入れる
1. [Releases](../../releases) から `SondevTerminal-setup-<バージョン>.exe` をダウンロードして実行します
   - 管理者権限は要りません（`%LOCALAPPDATA%\Programs\Sondev Terminal` に入ります）
   - 署名していないので、初回は SmartScreen の警告が出ることがあります（「詳細情報」→「実行」）
2. スタートメニューの **Sondev Terminal** から起動します

必要なもの: Windows 10 / 11（WebView2 ランタイムは通常入っています）。
Claude にブラウザを操作させる機能を使うときだけ、Node.js（`npx`）が必要です。

シェルは PowerShell 7（`pwsh`）があればそれを、なければ Windows PowerShell を使います。
シェル連携（フォルダ・conda の通知、`claude` / `cdi` / `clist` / `dlist` / `web` などの関数）は自動で読み込まれます。

---

## 使い方

### 基本の流れ
1. **Ctrl+Shift+N** で新しいワークスペースを作り、`cd` でプロジェクトのフォルダへ移動します（ワークスペース名はフォルダ名になります。Ctrl+Shift+R で変更）
2. **Ctrl+D** で右に分割、**Ctrl+T** で新しいタブ
3. 片方で `claude` を起動して作業を頼み、もう片方で別の作業をします
4. Claude が止まったら丸が赤くなり、終わったら緑になります。**Ctrl+Shift+U** でそのペインへ
5. 開発サーバーを動かしているなら **Ctrl+Shift+L** → URL 欄に `3000` で、画面を横で確認できます

### よく使うシェルのコマンド
| コマンド | 動作 |
|---|---|
| `claude` | Claude Code を起動（状態の表示・会話の再開・ブラウザ操作が自動で有効） |
| `codex` / `gemini` / `aider` など | そのエージェントを起動（状態の表示と完了の通知が有効） |
| `cdi [キーワード]` | よく行くフォルダの履歴から選んで移動 |
| `clist [キーワード]` | conda の環境を一覧から選んで有効にする（今の環境には `*`） |
| `dlist [キーワード]` | Docker のコンテナを一覧から選んで中のシェル（bash / sh）に入る。止まっているコンテナは起動する |
| `web [URL / ポート / 検索語]` | 同じ区画にブラウザのタブを開く（`web 3000` → localhost:3000、`web github.com`、`web rust 非同期`） |
| `sondev notify [--title 題] 本文` | 通知を出す（例: `cargo build; sondev notify ビルド完了`） |

### マウス
| 操作 | 動作 |
|---|---|
| ドラッグ / ダブルクリック / トリプルクリック | 文字の選択 / 単語 / 行 |
| 右クリック | 選択があればコピー、なければ貼り付け |
| Ctrl+クリック | URL をブラウザで、ファイルパスをエディタで、フォルダをエクスプローラーで開く（Ctrl を押すと下線が出ます） |
| Ctrl+Shift+クリック | URL を Sondev の中のブラウザで開く |
| タブをドラッグ | 並べ替え・別の区画へ移動。区画の端に落とすと分割して配置、サイドバーのワークスペースに落とすとそこへ移動 |
| 境界の線をドラッグ | 分割の大きさを変える |
| サイドバーのワークスペースをドラッグ | 並べ替え |
| サイドバーで右クリック | 名前を変更 / 新しいワークスペース / 上へ・下へ移動 / 削除 |
| ファイル一覧のフォルダ / ファイルをクリック | フォルダを開く・閉じる / ファイルを Sondev の中のタブで表示 |
| ファイル一覧のファイルを Ctrl+クリック | エディタで開く |
| ファイルの表示でドラッグ / トリプルクリック | 文字の選択 / 1 行の選択（Ctrl+C でコピー、Ctrl+A ですべて選択） |

Claude Code の全画面表示や vim など、アプリがマウスを使うときはクリックをアプリに渡します（ボタンが押せます）。
そのときに文字を選んでコピーしたいときは **Shift を押しながら** ドラッグしてください。

---

## ショートカット

迷ったら **Ctrl+Shift+P**（コマンドパレット）。すべての操作を名前で探せて、今のキーも表示されます。

### ワークスペース
| キー | 動作 |
|---|---|
| Ctrl+Shift+N | 新しいワークスペース |
| Ctrl+1 〜 Ctrl+8 / Ctrl+9 | 1〜8 番目 / 最後のワークスペースへ |
| Ctrl+Shift+PageDown / PageUp | 次 / 前のワークスペース |
| Ctrl+Shift+R | ワークスペース名の変更（サイドバーのダブルクリックでも可） |
| Ctrl+Shift+B | サイドバーの表示 / 非表示 |
| Ctrl+Shift+X | 右側のファイル一覧の表示 / 非表示（次の起動でも同じ） |

### タブ・分割
| キー | 動作 |
|---|---|
| Ctrl+T（Ctrl+Shift+T） | 新しいタブ（今のフォルダで開く） |
| Ctrl+W（Ctrl+Shift+W） | タブを閉じる |
| Ctrl+] / Ctrl+[（Ctrl+Tab / Ctrl+Shift+Tab） | 次 / 前のタブ |
| Ctrl+D | 右に分割（左右に並ぶ。縦の境界線ができる） |
| Ctrl+Shift+D（Ctrl+Shift+E） | 下に分割（上下に並ぶ。横の境界線ができる） |
| Ctrl+矢印 / Alt+矢印 | 隣の区画へ移動 |
| Alt+Shift+矢印 | 分割の境界を動かす |

各区画のタブ行の右にある **◫**（右に分割）・**⊟**（下に分割）のボタンでも分割できます。
境界の線はマウスでドラッグしても動かせます。

### ブラウザ
| キー | 動作 |
|---|---|
| Ctrl+Shift+L | 右に分割してブラウザを開く（ブラウザを見ているときは URL 欄へ） |
| Ctrl+Shift+O | 今の区画にブラウザのタブを開く |

### ターミナル
| キー | 動作 |
|---|---|
| Ctrl+Shift+F | スクロールバックを検索（Enter / ↑ で上へ、Shift+Enter / ↓ で下へ、Esc で閉じる） |
| Ctrl+C | 選択中ならコピー、そうでなければ中断（^C） |
| Ctrl+V | 貼り付け（画像だけのときは、Claude Code には画像の貼り付けとして渡します）。貼り付けた画像はターミナルの右上にサムネイルで出ます（クリックで大きく表示・Esc かクリックで閉じる、ホバーの ✕ で 1 つ消す、右端の ✕ で全部消す。Enter でプロンプトを送ると消えます） |
| Ctrl+Shift+C / Ctrl+Shift+V | コピー / 貼り付け |
| Ctrl+Enter / Shift+Enter | 改行（Claude Code の複数行入力など） |
| Ctrl+Shift+G | フォルダ履歴から移動（`cdi`） |
| Ctrl+Shift+A | conda の環境を選んで有効にする（`clist`） |
| Ctrl+= / Ctrl+- / Ctrl+0 | 文字を大きく / 小さく / 元に戻す |

### そのほか
| キー | 動作 |
|---|---|
| Ctrl+Shift+P | コマンドパレット |
| Ctrl+Shift+U | 入力待ち・終わったペインへ移動（押すたびに次へ） |
| Ctrl+, | 設定 |

> Ctrl+D・Ctrl+T・Ctrl+W・Ctrl+[・Ctrl+←→ は Sondev が使うので、シェルやアプリには届きません。
> シェルで使いたいキーがあれば、下の「キー割り当ての変更」で外せます。

---

## 設定

**Ctrl+,**（またはサイドバー右下の「⚙ 設定」）で開きます。テーマ・フォント・文字サイズ・行の高さ・カーソル・背景の透過・ブラウザ・Claude Code の再開・アップデートを変えられます。

設定は `%APPDATA%\SondevTerminal\settings.json` に保存されます。画面にない項目は、ここを直接編集します（再起動後に反映）。

### キー割り当ての変更
```json
"keybindings": {
  "ctrl-w": "",
  "ctrl-alt-w": "close_tab"
}
```
- 値はコマンドの名前です（`close_tab`、`new_tab`、`split_right` など。コマンドパレットの各項目に対応）
- 空の文字列にすると、そのキーの割り当てを外します（上の例では、Ctrl+W をシェルの単語削除に戻し、代わりに Ctrl+Alt+W でタブを閉じます）

### ファイルを開くエディタ
```json
"editor": "code -g {file}:{line}:{col}"
```
未設定なら VS Code → Cursor → Windsurf → 既定のアプリの順に探します。

---

## Claude にブラウザを操作してもらう

1. 設定の「AI からブラウザを操作できるようにする」を ON にして、Sondev を再起動します
2. Sondev のペインで `claude` を起動します。MCP「sondev-browser」が自動で使えるようになります（`/mcp` で確認できます）
3. Sondev でページを開いておき、「localhost:3000 のタブで、フォームに入力して送信して」のように頼みます

- `claude mcp add` での登録や、Chrome の起動は要りません（ポート 9222 は Sondev のブラウザが使っています）
- 操作の記録はプロジェクトのフォルダには書かれません
- ブラウザがログインしたままのサイトは、Claude があなたとして操作できます。必要なときだけ ON にするのがおすすめです

---

## 保存される場所

| 場所 | 中身 |
|---|---|
| `%APPDATA%\SondevTerminal\session.json` | ワークスペース・分割・タブ・各ペインのフォルダ・conda・Claude の会話 |
| `%APPDATA%\SondevTerminal\settings.json` | 設定 |
| `%APPDATA%\SondevTerminal\dirs.json` | フォルダ履歴（`cdi` 用） |
| `%APPDATA%\SondevTerminal\window.json` | ウィンドウの位置・大きさ |

ブラウザのデータ（ログイン状態など）は `%LOCALAPPDATA%\SondevTerminal\WebView2` にあります。
アンインストールしても、これらは残ります。以前の版の保存先（`%APPDATA%\wimux`）は、起動時に自動でここへ移します。

---

## 開発する人へ

```powershell
cargo run            # 開発ビルドで起動
cargo run --release  # 速度・メモリの確認
cargo test
```

必要なもの: Rust（MSVC ツールチェーン）、Visual Studio Build Tools（C++ デスクトップ開発＋Windows SDK）。
Linux でも型チェックとテストはできます（`RUSTFLAGS="--cfg wimux_check" cargo check`、`cargo check --target x86_64-pc-windows-msvc`）。

### インストーラーを作る
```powershell
powershell -ExecutionPolicy Bypass -File installer\build.ps1
```
`dist\SondevTerminal-setup-<version>.exe` ができます（Inno Setup 6 が必要: `winget install JRSoftware.InnoSetup`）。

### リリース
GitHub の Actions → **Release** → Run workflow でバージョン（例: `0.2.0`）を入れて実行すると、インストーラーを作って Releases に置きます。
タグを push しても同じです: `git tag v0.2.0 && git push origin v0.2.0`

リリースにせず試しにインストーラーだけ作りたいときは、Actions → **CI** → Run workflow（ブランチを選べます）。できたものは、その実行の「Artifacts」から取れます。

設計・内部の仕組み・ロードマップは [CLAUDE.md](CLAUDE.md) を参照してください。
