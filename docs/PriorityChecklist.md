# Priority Checklist (Agent Memory)

このファイルは、このリポジトリにおける **優先度(P0〜)** の定義と進捗を固定し、以後の会話/セッションでAIが番号を取り違えないための“記憶”として使う。

- ルール: ここに書かれていない「P番号」は使わない。
- ルール: 仕様がHaskellと異なる可能性がある項目は **P3(スキップ可)** に入れ、ユーザー指示がない限り実装しない。

Last updated: 2026-01-09

---

## P0 — End-to-end smoke tests (import traversal)
目的: 実行系が「複数ファイル」「import traversal」「export/import境界」「qualified ref」まで end-to-end で動くことを最優先で担保。
- [x] 既存のCLIスモーク群で概ね担保（`src/cli.rs`）

## P1 — Exceptions via IO (throw/catch/try)
目的: `throw/catch/try` をIOレイヤで実装し、伝播/捕捉/try(Either化)をスモークで保証。
- [x] 実装・テスト完了（commit: `4d0c477`）

## P2 — Braces / Semicolons surface syntax
目的: 既存インデントブロックに加えて、最低限の brace/`;` 形式を受理。
対象:
- `do { ...; ... }`
- `let a = ...; b = ... in ...`
- `where { a = ...; b = ... }`
- [x] 実装・テスト完了（commit: `4d0c477` + 追加テスト/修正）

## P3 — Haskellと違う可能性がある仕様 (スキップ可)
目的: 仕様書に書いてあっても、Haskellと差が出そう/設計が曖昧なら勝手に実装しない。
候補(例):
- `%VarName` 形式の型変数（docsにはあるが、方針によりユーザー明示指示がない限り実装しない）
- その他、文法・意味論でHaskellと差がありうるもの
- [ ] 原則スキップ（※誤って実装した分は revert 済: `4996536`）

## P4 — Numeric/Doc consistency (MVP)
目的: 現状実装(ランタイム/stdlib)とdocsの齟齬を無くし、危険な挙動(オーバーフロー等)をMVPとして安全側へ。
内容:
- Integer演算を checked 化し overflow をランタイムエラーに
- docsの String/Integer のMVP仕様を現実装に合わせて整合
- [x] 完了（commit: `94a57f5`）

## P5 — Backend numeric types + checked casts at boundaries (次の実装対象)
目的: docs(ImplementationPlan/TypeSystem/IR)にある「LLVM-aligned backend numeric types」と「境界でのchecked cast」を、MVPとして実装可能な範囲で入れる。
範囲(案):
- 型表現に `i32/i64` や `f32/f64` 相当(内部用)を導入（表面構文は最小限でも良い）
- リテラル/FFI境界（※FFI自体は後続でも可）での checked cast をIR/ランタイムで表現し、失敗時は実行時エラー
- テスト: cast成功/失敗(overflow/invalid)のスモーク

Status: 完了（MVP）
- [x] ランタイム値: `Integer`/`Float64` を `i64`/`f64` として保持（リテラル境界でparse、失敗は実行時エラー）
- [x] 注釈境界: `(:: i32/i64/f32/f64)` を checked cast としてIRに残し、失敗は実行時エラー
- [ ] 追加の境界(FFIなど)は後続

## P6 — Minimal FFI boundary (unsafe-free scaffold)
目的: 本物のC ABI呼び出しは `unsafe` を要求しうるため、まずは **FFI境界の振る舞い（引数/戻りのchecked cast）** を builtin でスモークできる形にする。
範囲:
- `ffiAddI32 :: i32 -> i32 -> i32` など、backend numeric types を要求する builtin を追加
- 呼び出し境界で range/overflow を検査し、失敗時は runtime error
- テスト: 正常系・引数out-of-range・演算overflow

Status: 完了（MVP）
- [x] `ffiAddI32`/`ffiAddF32` builtin を追加
- [x] 呼び出し境界で checked range/overflow
- [x] スモークテスト追加

---

## P7 — Unsafe boundary isolation + tracing
目的: 必要最小限の `unsafe`（FFI/特殊最適化/BigInt等）を **feature flag配下に隔離**し、デバッグ時に「unsafeが使われた」ことを追えるようにする。
- 実装: `--features unsafe_ffi/unsafe_bigint` 等で有効化（通常ビルドはoff）
- 観測: `KSCR_DEBUG_UNSAFE=1` で実行すると unsafe 境界通過を stderr に出す

Status: 完了（MVP）
- [x] feature flag を追加（unsafe_ffi / unsafe_bigint）
- [x] `KSCR_DEBUG_UNSAFE=1` で 1回だけタグ出力

## P8 — Optional BigInt Integer backend
目的: `Integer` を任意精度に戻したい/検証したい場合に、unsafe依存を **feature flag配下** に隔離した上で利用できるようにする。
範囲:
- `--features unsafe_bigint` のとき `Integer` を `BigInt` として解釈
- 既存の境界（`:: i32/i64` や ffiAddI32 等）で range check が効くこと
- テスト: 巨大整数の加算が通ること（通常ビルドではエラーになること）

Status: 完了（MVP）

## Notes
- 以後「P5を実装」と言われたら **このファイルのP5** を実装する。
- 新しい優先度が必要になったら、このファイルを更新してから着手する。
