<div align="center">
  <img src="assets/readme/hero.webp" width="1200" alt="AI ネイティブのダウンロードマネージャー OnesDM。急降下するハヤブサの軌跡が並列ダウンロードの列に変わる。">
  <h1><picture><source media="(prefers-color-scheme: dark)" srcset="assets/onesdm.svg"><img src="assets/onesdm-light.svg" width="36" alt=""></picture> OnesDM</h1>
  <p>Ones Download Manager · PlayOnes · AI-Native Download Manager</p>
  <p><strong>Any Agent. All Downloads.</strong></p>
  <p>Rust と GPUI で開発した、Windows・macOS・Linux 対応のダウンロードマネージャーです。メモリ安全な ODM Core に、プラグインと AI クライアントを組み合わせられます。</p>
  <p><a href="https://github.com/AreChen/OnesDM/releases/latest"><strong>ダウンロード</strong></a> · <a href="https://odm.playones.com/ja/">odm.playones.com</a></p>
</div>

<p align="center"><a href="README.md">English</a> · <a href="README.zh-CN.md">简体中文</a> · <a href="README.zh-Hant.md">繁體中文</a> · <strong>日本語</strong> · <a href="README.ko.md">한국어</a> · <a href="README.es.md">Español</a> · <a href="README.pt-BR.md">Português (Brasil)</a> · <a href="README.fr.md">Français</a> · <a href="README.de.md">Deutsch</a></p>

## 機能

- HTTP(S) の並列ダウンロードと再開、FTP(S)、SFTP に対応します。
- HLS と静的 DASH を保存できます。ブラウザーで検出した音声や動画を検索し、まとめてダウンロードできます。
- ファイル名・配信元・URL で検索でき、種類別の絞り込みや Shift による範囲選択、ファイルを開く操作ができます。
- 保存先とキャッシュの場所を指定し、キャッシュの消去やタスク別のプロキシ設定ができます。
- 独立した進捗ウィンドウとライト・ダークテーマを用意しています。初回はシステムの対応言語を使い、「設定 → 外観と言語」で 9 言語から選択できます。

## スクリーンショット

### ライトテーマ

[![ライトテーマの OnesDM のタスクリスト。完了したダウンロードが4件並んでいる。](assets/readme/downloads-light.webp)](assets/readme/downloads-light.png)

### ダークテーマ

[![ダークテーマの OnesDM のタスクリスト。完了したダウンロードが4件並んでいる。](assets/readme/downloads-dark.webp)](assets/readme/downloads-dark.png)

## ダウンロード

[GitHub Releases からインストーラーを入手](https://github.com/AreChen/OnesDM/releases/latest)

| プラットフォーム | パッケージ | インストール方法 |
| --- | --- | --- |
| Windows · x64 | `OnesDM.exe` | インストーラーを実行します。 |
| macOS 12+ · Apple Silicon | `OnesDM-*-macos-arm64.dmg` | OnesDM を「アプリケーション」にドラッグします。 |
| macOS 12+ · Intel | `OnesDM-*-macos-x64.dmg` | OnesDM を「アプリケーション」にドラッグします。 |
| Linux · x86-64 | `OnesDM-*-linux-x64.tar.gz` | [必要なライブラリ](desktop/packaging/linux/README.md)をインストールし、アーカイブを展開して `ndm-desktop` を実行します。 |

**0.6.8、0.6.9、0.7.0 からの更新**　これらの版は現在の GitHub アドレスのパッケージを検出できません。ダウンロードを一時停止するか完了させ、アプリを完全に終了してから 0.7.1 以降を一度手動でインストールしてください。既存のプロファイルを残すと履歴・設定・プラグインを引き継げます。

以後はアプリで更新を確認できます。更新パッケージのサイズと SHA-256 を検証し、Windows はタスク完了後にインストールします。macOS はアプリを置き換え、必要な場合は DMG を開きます。Linux はアーカイブのフォルダーを開きます。

新しいプロファイルでは HTTP ダウンロード 1 件につき最大 16 接続です。ファイルサイズと初回応答時間に応じてセグメントの大きさを決めます。既存の設定は引き継ぎます。

## コマンドライン

インストール先に含まれる `onesdm` を PATH に追加するか、フルパスで実行します。macOS では `OnesDM.app/Contents/MacOS` にあります。HTTP(S) の URL をデスクトップに送るか、標準入出力経由で MCP を起動できます。

```text
onesdm --version
onesdm download <URL>
onesdm mcp
```

## プラグインと AI

<img src="assets/readme/plugins-ai.webp" width="1200" alt="フィルム、音声波形、ピアネットワーク、端末のプロンプトがダウンロードの流れに合流し、ファイルの束へ続く。">

プラグインマーケットの **BitTorrent** は .torrent とマグネットリンクに、**Media Downloader** は対応サイトの動画・音声・プレイリストに対応します。 [GitHub](https://github.com/AreChen/onesdm-plugin-market)

OpenAI Codex（CLI・デスクトップ）、Claude Code、Pi、OpenCode、DeepSeek Harness から OnesDM MCP と Skill を使ってタスクを追加・管理できます。 [プラグインと AI の設定](docs/plugins-and-ai.md)

## ブラウザー拡張機能

Chrome、Edge、Brave で種類・サイズ・ドメインに応じてダウンロードを引き継ぎます。メディア横のボタンやリソース一覧から一括取得もできます。各リリースの拡張機能 ZIP を使い、接続先に OnesDM を選択してください。Neat Download Manager にも対応します。 [拡張機能のインストールガイド](docs/extension-install.md)

[Windows](desktop/packaging/windows/README.md#browser-extension) · [macOS](desktop/packaging/macos/README.md#browser-extension) · [Linux](desktop/packaging/linux/README.md#browser-extension)

<details>
<summary><strong>ビルドとリリース</strong></summary>

desktop-vX.Y.Z タグをプッシュすると、GitHub Actions が Windows、macOS 両アーキテクチャ、Linux と拡張機能をビルドし、テスト成功後に公開します。 [GitHub Actions](.github/workflows/release.yml)

[Windows](desktop/packaging/windows/README.md) · [macOS](desktop/packaging/macos/README.md) · [Linux](desktop/packaging/linux/README.md)

</details>

---

[MIT](LICENSE) · [プライバシー](PRIVACY.md) · Neat Download Manager と提携していない独立プロジェクトです。
