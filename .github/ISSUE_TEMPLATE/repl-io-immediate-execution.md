---
name: "REPL: IO 式を即時実行する"
about: "REPL で `IO a` の式を入力したときに、`Show (IO a)` を要求せずに実行する（GHCI 風）"
title: "REPL: IO 式の即時実行（Show (IO a) を要求しない）"
labels: ["repl", "runtime", "typeclass"]
---

## 現象

REPL で `IO` の式を評価すると、値を表示するために `Show (IO a)` 制約が要求されて失敗する。

例:

```
> :t stdoutWrite
it : String -> IO Unit

> stdoutWrite "1234\n"
error: cannot satisfy constraint: Show IO Unit
```

## 期待する挙動

GHCI に近い体験として、REPL で `IO a` の式が入力された場合は **即時に実行** される。

- `it : IO Unit` の場合
  - 実行して終了（追加で `Show (IO Unit)` は要求しない）
- `it : IO a`（`a` が `Unit` 以外）の場合
  - 実行し、結果 `a` を表示する（`Show a` は必要）

## 原因メモ（実装のあたり）

REPL は内部的に一時モジュールを生成して `main` を組み立てているが、現在は常に

```
main = stdoutWrite (toString it ++ "\\n")
```

の形になっており、`it` が `IO a` のときに `Show (IO a)` を要求してしまう。

該当: src/cli.rs の `build_repl_module_src`。

## 受け入れ条件

- 上記の再現手順がエラーにならず、`stdoutWrite` が実行される
- 既存の REPL 挙動（純粋式の `it` 表示など）を壊さない
- `cargo test` が通る

## 追加テスト案

- `stdoutWrite "x"` が実行できる（`Show (IO Unit)` 不要）
- `do { stdoutWrite "x"; return 42 }` のような `IO Integer` が実行でき、`42` が表示できる
