# テスト実行パフォーマンス最適化戦略

## 概要

テストの実行時間が長すぎる問題に対処するため、ファイル・モジュール・フェーズごとにASTと型推論結果をキャッシュする包括的な戦略を文書化しました。

**現在のパフォーマンス:**
- 単一テスト実行: 約5秒
- 全テストスイート: 5分以上（282テスト）
- 主なボトルネック: stdlib ClassEnv読み込み、モジュールimportの平坦化、型推論

**最適化後の予想パフォーマンス:**
- 単一テスト実行: 0.5-1秒（5-10倍の高速化）
- 全テストスイート: 1-2分（3-5倍の高速化）

## 主な問題点の特定

### 1. Stdlib ClassEnv読み込み - 最重要 🔴

**場所:** `src/types.rs:4824-4875` (`load_stdlib_class_env()`)

**問題:**
- **全ての** `typecheck_file()` 呼び出しで実行される
- `stdlib/` ディレクトリを再帰的に走査
- stdlib内の全ての `.ks` ファイルをパース
- クラスとインスタンス宣言を収集
- 時間計算量: O(F × C) ここでF = ファイル数、C = 平均宣言数

**影響:** テスト実行時間の約50-80%

**現在のコード:**
```rust
fn load_stdlib_class_env() -> Result<ClassEnvIndex> {
    let stdlib = stdlib_root();
    let mut entries = Vec::new();
    for entry in walkdir::WalkDir::new(&stdlib) {
        // 毎回全てのファイルをパース
        let entry = entry?;
        if entry.file_type().is_file() && entry.path().extension() == Some("ks".as_ref()) {
            entries.push(entry.path().to_path_buf());
        }
    }
    // ... 全てのファイルをパース ...
}
```

### 2. モジュール型チェックハッシュ計算 - 高優先度 🟡

**場所:** `src/types/stdlib_cache.rs:23-33` (`hash_module_ast()`)

**問題:**
```rust
pub(super) fn hash_module_ast(module: &ast::Module) -> u64 {
    let mut hasher = DefaultHasher::new();
    let module_str = format!("{:?}", module); // AST全体をフォーマット！
    module_str.hash(&mut hasher);
    hasher.finish()
}
```

- AST全体をデバッグ文字列としてフォーマット: O(ASTサイズ)
- 一時的な文字列割り当てを作成
- 全ての型チェック呼び出しでハッシュルックアップが発生

**影響:** キャッシュルックアップ時間の約10-20%

### 3. Import平坦化がキャッシュされていない - 中優先度 🟡

**場所:** `src/types.rs:4538-4570` (`load_module_with_imports_ast_with_loader()`)

**問題:**
- 各 `typecheck_file()` 呼び出しで空のキャッシュを持つ新しい `ModuleLoader` を作成
- `collect_imports()` が全てのimportを再帰的に処理
- `qualify_items()` がASTを走査してモジュール接頭辞で名前を修飾
- 結果がテスト実行間でキャッシュされない

**影響:** マルチモジュールプロジェクトで約10-30%

### 4. 制約の簡略化 - 低優先度 🟢

**影響:** 複雑な型階層で約5-15%

## 提案するキャッシュ戦略

### フェーズ1: Stdlib ClassEnvキャッシュ（優先度1）🔴

**目標:** stdlib ClassEnvをグローバルにキャッシュし、stdlib変更時のみ無効化する。

**実装アプローチ:**

1. **Stdlibコンテンツハッシュを追加**

```rust
// In src/types/stdlib_cache.rs

#[derive(Clone, Debug)]
struct CachedStdlibClassEnv {
    content_hash: u64,
    class_env: ClassEnvIndex,
}

static STDLIB_CLASS_ENV_CACHE: OnceLock<Mutex<Option<CachedStdlibClassEnv>>> = OnceLock::new();
```

2. **コンテンツハッシュを計算**

全てのstdlibファイルを効率的にハッシュ化:

```rust
pub(super) fn compute_stdlib_content_hash() -> Result<u64> {
    let stdlib = stdlib_root();
    let mut hasher = DefaultHasher::new();
    
    for entry in walkdir::WalkDir::new(&stdlib).sort_by_file_name() {
        let path = entry?.path().to_path_buf();
        if path.extension() == Some("ks".as_ref()) {
            path.hash(&mut hasher);
            // ファイルメタデータをハッシュ化（高速検証用）
            if let Ok(meta) = std::fs::metadata(&path) {
                meta.modified().ok().hash(&mut hasher);
                meta.len().hash(&mut hasher);
            }
        }
    }
    Ok(hasher.finish())
}
```

3. **ClassEnvをグローバルにキャッシュ**

```rust
pub(super) fn load_stdlib_class_env_cached() -> Result<ClassEnvIndex> {
    let current_hash = compute_stdlib_content_hash()?;
    
    if let Ok(mut cache) = stdlib_class_env_cache().lock() {
        if let Some(cached) = &*cache {
            if cached.content_hash == current_hash {
                return Ok(cached.class_env.clone()); // キャッシュヒット！
            }
        }
        
        // キャッシュミスまたは無効 - 再構築
        drop(cache); // 高コスト操作中はロック解放
        let class_env = load_stdlib_class_env_uncached()?;
        
        if let Ok(mut cache) = stdlib_class_env_cache().lock() {
            *cache = Some(CachedStdlibClassEnv {
                content_hash: current_hash,
                class_env: class_env.clone(),
            });
        }
        
        Ok(class_env)
    } else {
        load_stdlib_class_env_uncached()
    }
}
```

**期待される改善:** 3-5倍の高速化（単一テスト: 1-1.5秒）

### フェーズ2: モジュール型チェックハッシュの最適化（優先度2）🟡

**目標:** デバッグフォーマットの代わりに構造的ASTハッシュを使用する。

**実装アプローチ:**

AST型に `#[derive(Hash)]` を追加:

```rust
// In src/ast.rs
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Module {
    pub name: Option<String>,
    pub items: Vec<Item>,
}
```

ハッシュ化を簡素化:

```rust
// In src/types/stdlib_cache.rs
pub(super) fn hash_module_ast(module: &ast::Module) -> u64 {
    let mut hasher = DefaultHasher::new();
    module.hash(&mut hasher);  // 直接ハッシュ化！
    hasher.finish()
}
```

**期待される改善:** 追加で10-20%の高速化

### フェーズ3: Import平坦化のキャッシュ（優先度3）🟡

**目標:** import平坦化されたモジュールをメモ化して、冗長な修飾を回避する。

**実装アプローチ:**

Import平坦化キャッシュを追加:

```rust
#[derive(Clone)]
struct CachedFlattenedImport {
    source_hash: u64,
    items: Vec<ast::Item>,
}

static IMPORT_FLATTEN_CACHE: OnceLock<Mutex<HashMap<PathBuf, CachedFlattenedImport>>> = 
    OnceLock::new();
```

**期待される改善:** マルチモジュールテストで追加10-30%

### フェーズ4: 制約簡略化のキャッシュ（優先度4）🟢

派生インスタンス制約（Show、Eq、Ord）をメモ化。

**期待される改善:** 複雑な型階層で追加5-15%

## 実装ロードマップ

### ステップ1: ベースライン測定 ⏱️

最適化前のベースラインメトリクスを確立:

```bash
time cargo test --lib cli_impl::tests::cli_run_command_smoke
```

期待されるベースライン:
- 単一テスト: 3-5秒
- 50 CLIテスト: 150-250秒

### ステップ2: フェーズ1を実装（Stdlib ClassEnvキャッシュ）🔴

**変更するファイル:**
1. `src/types/stdlib_cache.rs` - `CachedStdlibClassEnv`、ハッシュ関数を追加
2. `src/types.rs` - `typecheck_with_stdlib_class_env()` 呼び出し箇所を更新

**期待される改善:** 3-5倍の高速化（単一テスト: 1-1.5秒）

### ステップ3: フェーズ2を実装（ハッシュ最適化）🟡

**期待される改善:** 追加10-20%の高速化

### ステップ4: フェーズ3を実装（Import平坦化）🟡

**期待される改善:** マルチモジュールテストで追加10-30%

### ステップ5: 最終パフォーマンス測定 ⏱️

最終的な期待パフォーマンス:
- 単一テスト: 0.5-1秒（5-10倍の改善）
- 50 CLIテスト: 25-50秒（3-5倍の改善）
- 282テスト全体: 1-2分（3-5倍の改善）

## キャッシュ管理

### メモリ使用量

キャッシュごとの推定メモリフットプリント:

| キャッシュ | エントリあたりサイズ | 最大エントリ数 | 合計 |
|-------|---------------|-------------|-------|
| Stdlib AST | ~50-500 KB | ~50ファイル | 2-25 MB |
| Stdlib ClassEnv | ~100-500 KB | 1 | 100-500 KB |
| Module Typecheck | ~10-100 KB | ~100モジュール | 1-10 MB |
| Import Flatten | ~50-200 KB | ~50モジュール | 2-10 MB |

**合計推定:** 5-45 MB（開発用として許容可能）

### キャッシュクリア

開発用に、キャッシュをクリアするCLIコマンドを追加:

```rust
// In src/cli_impl.rs
"clear-cache" => {
    types::stdlib_cache::clear_all_caches();
    println!("全てのキャッシュをクリアしました。");
    Ok(())
}
```

### 環境変数

デバッグ用の環境変数を追加:

```bash
# 全てのキャッシュを無効化
KSCR_NO_CACHE=1 cargo test

# キャッシュ統計を表示
KSCR_CACHE_STATS=1 cargo test

# 実行前にキャッシュをクリア
KSCR_CLEAR_CACHE=1 cargo test
```

## テスト戦略

### ユニットテスト

各キャッシュ実装には以下が必要:

1. **キャッシュヒットテスト** - キャッシュされた結果が新規計算と一致することを確認
2. **キャッシュミステスト** - キャッシュが変更を正しく検出することを確認
3. **キャッシュ無効化テスト** - 関連する変更でキャッシュが無効化されることを確認
4. **並行性テスト** - 並列テスト実行でキャッシュが機能することを確認

### 統合テスト

キャッシュ有効で既存のテストスイートを実行:

```bash
# 全てのテストを実行 - キャッシュ有効でパスするはず
cargo test

# キャッシュ無効で実行 - 依然としてパスするはず
KSCR_NO_CACHE=1 cargo test

# キャッシュ統計で実行 - ヒット率を測定
KSCR_CACHE_STATS=1 cargo test 2>&1 | tee cache_stats.log
```

## リスクと軽減策

### リスク1: キャッシュ無効化バグ

**リスク:** ソース変更時にキャッシュが無効化されず、古い結果が返される。

**軽減策:**
- ファイルメタデータ（mtime + サイズ）を検証に使用
- 重要なキャッシュにコンテンツハッシュを追加
- `KSCR_NO_CACHE` エスケープハッチを提供
- フォーマット変更を検出するためキャッシュバージョン番号を追加

### リスク2: メモリ不足

**リスク:** 長時間実行プロセスでキャッシュが無制限に成長する。

**軽減策:**
- 大きなキャッシュにLRU削除を実装
- 最大キャッシュサイズ制限を設定
- 手動キャッシュクリアコマンドを提供
- CLI/テスト使用のキャッシュサイズは許容可能（~5-45 MB）

### リスク3: ハッシュ衝突

**リスク:** 異なるASTが同じ値にハッシュされ、不正なキャッシュヒットが発生。

**軽減策:**
- 64ビットハッシュを使用（2^64空間、衝突は unlikely）
- 重要なキャッシュでは、ハッシュマッチ後にコンテンツを検証
- 必要に応じてstdlib ClassEnvに暗号学的ハッシュ（SHA-256）を使用

### リスク4: 既存テストの破壊

**リスク:** キャッシュ変更が既存のテスト動作を破壊する。

**軽減策:**
- 全てのテストは `KSCR_NO_CACHE=1` でパスする必要がある
- 全てのテストはキャッシュ有効でパスする必要がある
- キャッシュ固有のテストを追加
- テスト出力の差異を慎重にレビュー

## 結論

このキャッシュ戦略は、以下によってテスト実行時間を体系的に最適化するアプローチを提供します:

1. **stdlib ClassEnvをグローバルにキャッシュ**（50-80%高速化） - 優先度1
2. **ハッシュ計算の最適化**（10-20%高速化） - 優先度2
3. **import平坦化のキャッシュ**（10-30%高速化） - 優先度3
4. **制約簡略化のキャッシュ**（5-15%高速化） - 優先度4

期待される全体的な改善: **個別テスト実行が5-10倍高速化**、**全テストスイートが3-5倍高速化**。

戦略は保守的で段階的であり、慎重なキャッシュ無効化を通じて正確性を維持します。各フェーズは独立して実装・検証できます。

---

完全な詳細（英語）については、`docs/CACHING_STRATEGY.md` を参照してください。
