# Sondev Terminal

Windows 向けの軽量なワークスペース型ターミナル（開発中。開発名は wimux）。

- 左サイドバーにワークスペースを並べて切り替え
- ペインごとのフォルダ・conda 環境・Claude Code の状態を覚えて、次回そのまま再開
- AI エージェントの進捗を通知
- GPUI ＋ alacritty_terminal ＋ ConPTY によるネイティブ実装

## 起動

```powershell
cargo run
```

必要なもの: Rust（MSVC ツールチェーン）、Visual Studio Build Tools（C++ デスクトップ開発＋Windows SDK）。

シェル連携（`shell-integration/wimux.ps1`）は自動で読み込まれます。

キー操作の一覧は [CLAUDE.md](CLAUDE.md#キー操作) を参照。

## インストーラーを作る

```powershell
powershell -ExecutionPolicy Bypass -File installer\build.ps1
```

`dist\SondevTerminal-setup-<version>.exe` ができます（Inno Setup 6 が必要: `winget install JRSoftware.InnoSetup`）。

開発方針・ロードマップは [CLAUDE.md](CLAUDE.md) を参照。
