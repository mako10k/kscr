# kscr LSP / VSIX 設計（推奨案）

この文書は、kscr の VS Code 拡張を **LSP 対応まで拡張**していくための、推奨アーキテクチャと段階的ロードマップをまとめた設計書です。

- 対象: kscr 言語（`.ks`）
- 現状: TextMate 文法 + language-configuration のみ（MVP）
- 目標: kscr 実装（lexer/parser/typechecker 等）と整合する LSP を提供し、VSIX を「入れたらすぐ便利」な品質にする

---

## 1. 要求と設計の前提

### 1.1 最優先の価値

- **未保存バッファでも正しい診断**（パース/型/モジュール解決）
- **ホバーで型が見える**
- **定義ジャンプができる**（特に import/モジュール境界を含む）
- 大規模化しても破綻しない（キャッシュ/増分の導入余地）

### 1.2 kscr 固有の前提（守る）

- import の基準は「**import 元ファイルのディレクトリ相対**」
- Prelude は「暗黙 import」がある（ただし明示 import がある場合の扱いに注意）
- エラーは `src/error.rs` に集約（`Error::Msg(String)` / `Error::Io(std::io::Error)`）
- main crate は unsafe 禁止（`#![forbid(unsafe_code)]`）

---

## 2. 推奨方針（結論）

### 2.1 LSP サーバー実装言語

- **Rust 実装を推奨**（kscr の parser/typechecker を直接リンクする）
- LSP ライブラリは `tower-lsp` を第一候補

理由:
- kscr の解析機能（lexer/parser/types）を安全に再利用できる
- 位置情報/モジュール解決などを “言語側の真実” に寄せられる
- TS 側は薄いクライアントにでき、長期保守が楽

### 2.2 配布/起動方式（VSIX を充実させるための推奨）

**推奨: B + C のハイブリッド**

- **B: 初回起動時に LSP サーバーバイナリを自動ダウンロード**（GitHub Releases など）
- **C: `kscr.lsp.serverPath` 設定でローカルの `kscr-lsp` を優先利用可能**

メリット:
- ユーザーは “入れたら動く”
- 開発者/CI はローカルビルドを即利用できる
- VSIX の巨大化（A: 同梱）を回避できる

注意点:
- 企業環境/オフラインでの失敗に備え、設定での回避（C）と分かりやすいエラーメッセージが必須

---

## 3. MVP 機能セット（LSP）

最初に実装して「体験が変わる」最小セットを以下とする。

### 3.1 必須（Phase 1）

- `textDocument/publishDiagnostics`
  - パースエラー
  - import 解決エラー（見つからない/循環参照など）
  - 型エラー（制約不一致/未解決シンボル等）
- `textDocument/hover`
  - 変数/関数/コンストラクタ/型名の推論結果（可能なら）
- `textDocument/definition`
  - トップレベル定義（モジュール境界を跨ぐ）
- `textDocument/documentSymbol`
  - モジュール内の項目一覧（関数/データ/型別名/型クラス/インスタンス）

### 3.2 後回し（Phase 2+）

- 補完（`completion`）: スコープ/インポート/型に依存し、設計が重い
- 参照検索（`references`）: def-use の安定が前提
- リネーム（`rename`）: 境界/衝突/エクスポートの扱いが必要
- Semantic Tokens: 正確だが解析コストと仕様の詰めが必要

---

## 4. コンポーネント設計

### 4.1 リポジトリ構成案

- 新規: `crates/kscr_lsp/`（またはトップ直下 `src/lsp/` だが、分離クレート推奨）
  - バイナリ: `kscr-lsp`
  - 依存: `kscr`（現 main crate）をライブラリとして利用

### 4.2 VS Code 拡張（TS）側

- 役割: Language Client + サーバーダウンロード/起動管理
- 推奨ライブラリ:
  - `vscode-languageclient`
- 追加コマンド:
  - Restart kscr LSP
  - Show kscr LSP Logs

### 4.3 LSP サーバー（Rust）側

- 役割: LSP プロトコル処理、VFS、解析キャッシュ、診断/ジャンプ/hover 用のクエリ

内部を 3 層に分ける（責務を固定する）:

1) **Transport 層**: JSON-RPC / LSP
2) **Session 層**: ワークスペース、設定、ドキュメント管理、VFS
3) **Analysis 層**: kscr の lexer/parser/typechecker 等を呼び出す Facade

---

## 5. 解析モデル（VFS とモジュール解決）

### 5.1 Virtual File System（VFS）

VS Code の未保存バッファを扱うため、解析入力は OS の実ファイルではなく VFS を正とする。

- キー: `Url`（`file://...`）
- 値: テキスト内容、バージョン、改行情報
- 永続: メモリのみ（再起動で破棄）

方針:
- LSP から見える “現在の内容” を常に優先
- import 先が未オープンの場合は実ファイルを読む（ただしキャッシュ）

### 5.2 モジュール解決

kscr 仕様に合わせ、import 解決は以下。

- 基準ディレクトリ: **import 元ファイルの親ディレクトリ**
- `import Foo.Bar` → `<base>/Foo/Bar.ks` を探索（例）
- 循環 import は検出し、診断に落とす（既存実装があるなら再利用）

### 5.3 Prelude の扱い

- 言語実装と同じルールを厳守
- LSP では “暗黙 import されている名前” も解決/hover できるようにする

---

## 6. 診断（Diagnostics）設計

### 6.1 位置情報

- kscr 側の Span（開始/終了）を LSP の `Range` に変換
- 文字単位/UTF-16 などの差は LSP 側で吸収する
  - VS Code は UTF-16 基準が絡むため、実装での変換ユーティリティが必要

### 6.2 診断の分類

- Severity
  - parse/import/type は基本 `Error`
- Source
  - `kscr` 固定

可能なら:
- import 不足などの Quick Fix は Phase 3

---

## 7. キャッシュと増分（段階導入）

### Phase 1（正しさ優先）

- 変更ファイルを中心に「対象モジュール + 依存」を再解析/再型検査
- キャッシュは最小（ファイル内容とモジュール単位の結果）

### Phase 2（高速化）

- 依存グラフに基づく無効化
- AST/型付け結果のモジュールキャッシュ

### Phase 3（IDE 機能拡張）

- def-use インデックス
- references / rename / completion

---

## 8. VSIX の“充実”チェックリスト

### 8.1 設定（案）

- `kscr.lsp.enabled`: boolean（既定 true）
- `kscr.lsp.serverPath`: string | null（指定時はこれを優先）
- `kscr.lsp.download.enabled`: boolean（既定 true）
- `kscr.lsp.download.channel`: `stable` | `nightly`（既定 stable）
- `kscr.lsp.trace`: `off` | `messages` | `verbose`

### 8.2 UX

- OutputChannel: `Kscr Language Server`
- ステータスバー: サーバー起動状態（任意）
- 失敗時の案内:
  - 「自動DLに失敗しました。`kscr.lsp.serverPath` を設定してください」

---

## 9. リリース運用（推奨）

### 9.1 LSP サーバーバイナリ

- GitHub Releases に platform 別アセットを公開
  - Linux x64 / Linux arm64
  - macOS x64 / macOS arm64
  - Windows x64

### 9.2 VSIX

- VSIX は拡張本体（TS/設定/grammar）を配布
- 初回起動で該当 platform の `kscr-lsp` を取得

---

## 10. テスト方針

- Rust: LSP サーバーのユニットテスト（VFS/モジュール解決/診断変換）
- Rust: 既存 `tests/*.ks` を “解析入力” として再利用できる形を目指す
- VS Code: 手動スモーク（`example_hello.ks` を開いて diagnostics/hover/definition を確認）

---

## 11. ロードマップ（具体）

1) LSP サーバー最小骨格（initialize/shutdown/exit）
2) VFS + didOpen/didChange/didSave
3) diagnostics（parse/import/type）
4) hover
5) definition
6) documentSymbol
7) 自動DL + 設定（serverPath）
8) キャッシュ/増分の改善
9) completion → references → rename

---

## 付録: 命名案

- バイナリ: `kscr-lsp`
- VS Code 拡張設定 prefix: `kscr.lsp.*`
