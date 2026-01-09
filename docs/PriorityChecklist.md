# Priority Checklist (Agent Memory)

このファイルは、このリポジトリにおける **優先度(P0〜)** の定義と進捗を固定し、以後の会話/セッションでAIが番号を取り違えないための“記憶”として使う。

- ルール: ここに書かれていない「P番号」は使わない。
- ルール: 仕様がHaskellと異なる可能性がある項目は **P3(スキップ可)** に入れ、ユーザー指示がない限り実装しない。

Last updated: 2026-01-09

---

## P0 — End-to-end smoke tests (import traversal)
目的: 実行系が「複数ファイル」「import traversal」「export/import境界」「qualified ref」まで end-to-end で動くことを最優先で担保。

Status: 進行中（継続追加OK / ただし“実装済みを再実装”しないよう、下記の完了項目を基準にする）

### Done
- [x] run: import traversal（A→B→Main）を跨いで実行できる（commit: `34c439d`）
- [x] run: transitive import + qualified ref（`import A as OM; OM.x`）が動く（commit: `34c439d`）
- [x] run: `import A` が unqualified 参照（`x`）と module qualifier（`A.x`）の両方を許す（commit: `4904ba6`）
- [x] typecheck: export/import 境界が効き、未exportは入らない（commit: `4904ba6`）
- [x] run: `import A as A1` / `import B as B1` で同名の衝突を qualified で解消できる（commit: `f276e02`）
- [x] typecheck: cyclic imports を検出し、エラー文に `cyclic imports` が含まれる（commit: `f276e02`）

### Next
- [ ] import traversal: data/constructors + case + do を跨いだより実戦的なスモークを追加（必要になったら）

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

## P12 — Haskell風の関数clause/ガード (parser desugar)
目的: Haskell風の書き味（複数clause、ガード、let/where内clause）を AST 拡張せずにパーサで desugar して、既存の型推論/IR/ランタイム変更を最小化する。

Status: 完了（MVP）
- [x] top-level: 同名の関数clauseを集約し、単一binding（lambda + case）へdesugar（commit: `71c33a7`）
- [x] guard付き関数clause（`f x | guard = body`）を受理し `CaseArm.guard` に載せる（commit: `5bf35e1`）
- [x] let/where内でも同様に clause を集約して desugar（commit: `5bf35e1`）
- [x] `|` の曖昧性回避: 関数引数の or-pattern は括弧必須（例: `f (0 | 1) = ...`）（commit: `5bf35e1`）

## P13 — Imports/Exports を Haskell 寄せ（おすすめ順で実施）
目的: import/export まわりのHaskell的な「書き味」「名前解決の分かりやすさ」「仕様固定」を段階的に改善する。

Recommended order:
1) P13C（診断強化） → 2) P13D（探索/仕様固定） → 3) P13A（表面構文） → 4) P13B（export粒度）

- [x] **P13C: import名前解決の診断強化**（同名衝突の説明、候補提示、unknown qualifier で許容qualifier表示 など）
  - done: better conflict/qualifier errors + tests (commit: `bac0b11`)
- [x] **P13D: import探索の仕様固定**（探索順/モジュール名↔パスのルール化 + スモーク）
  - rule: try `<importer_dir>/<Module>.ks` then `<repo>/stdlib/<Module>.ks`; on miss, error shows tried paths
  - rule: imported module must declare `module <Module> where` (mismatch is error)
  - tests: local-over-stdlib shadowing, tried-paths in error, module name mismatch (commit: `7985565`)
- [x] **P13A: import文の表面構文をHaskell寄せ**（`import qualified A as OM` 等）
  - behavior: `import qualified` is qualified-only; `import A as OM` is unqualified+OM qualifier
  - tests: updated existing smokes + added unqualified+qualifier smoke (commit: `2699f9e`)
- [ ] **P13B: exportの粒度強化**（例: `export Maybe(..)` 等）

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
目的: `Integer` のセマンティクスは常に任意精度（自作safe backend）で揃えつつ、必要なら `num-bigint` を **feature flag配下** で使えるようにする（性能/検証用途）。
範囲:
- デフォルト: 自作の可変長Integer backend（unsafe無し）
- `--features unsafe_bigint`: `num-bigint` backend（optional dep / 別crate隔離）
- 既存の境界（`:: i32/i64` や ffiAddI32 等）で range check が効くこと

Status: 完了（MVP）

## P9 — Real C ABI FFI (unsafe isolated)
目的: 本物のC ABI呼び出しを **feature flag配下でunsafe隔離** しつつ、境界の振る舞い（String→C string、戻り値の型/範囲）をMVPとして確認できるようにする。

範囲(MVP):
- `--features unsafe_ffi` のときのみ有効な builtin を追加
  - 例: `ffiPuts :: String -> IO i32`（C標準ライブラリの `puts` を呼ぶ）
- 文字列境界: interior NUL はエラー
- `KSCR_DEBUG_UNSAFE=1` で `unsafe used: ffiPuts` を観測可能に
- テスト: feature付きで `ffiPuts` が実行できるスモーク

注意:
- `cfg(feature = "unsafe_ffi")` だけだと `cargo geiger` が unsafe 構文を検出してしまうため、**unsafeは別crate（optional dep）に隔離する** 方針で進める。
- これにより、デフォルトビルドの必須ゲート（`cargo geiger`）は維持しつつ、`--features unsafe_ffi` 時だけ unsafe を含む依存を有効化できる。

Status: 完了（MVP）
- [x] `ffiPuts` を `--features unsafe_ffi` のときのみ有効な builtin として追加
- [x] unsafe は別crate（optional dep） `kscr_unsafe_ffi` に隔離
- [x] feature付きスモークテスト追加

運用（ゲート）:
- デフォルト必須: `cargo test && cargo clippy -- -D warnings && cargo geiger && cargo +nightly udeps`
- 任意（unsafe_ffi有効時）: `cargo test --features unsafe_ffi` / `cargo geiger --features unsafe_ffi`

## P10 — Unsafe features gate policy
目的: `unsafe_ffi` / `unsafe_bigint` など **unsafeを含みうるfeature** を、CI/運用でどう検証するかを固定し、破綻しないルールにする。

方針(MVP):
- デフォルトビルド（feature無し）:
  - `kscr` 本体は `#![forbid(unsafe_code)]` を付与し、unsafeはそもそも書けない
  - 必須ゲート: `cargo test && cargo clippy -- -D warnings && cargo geiger && cargo +nightly udeps`
- unsafe feature（例: `unsafe_ffi` / `unsafe_bigint`）:
  - unsafeは別crate（optional dep）に隔離
  - 別ジョブで `cargo test --features ...` を回す
  - `cargo geiger --features ...` は「許容（ただし対象crateが増えない/件数が増えない）」を監視

Status: 完了（MVP）
- [x] 方針決定（上記）
- [ ] CI反映（このリポジトリのCI導入/更新が必要なら別タスク）

## P11 — Isolate BigInt backend into subcrate
目的: `unsafe_bigint`（任意精度Integer）を別crate（optional dep）に隔離し、`kscr` 本体から `num-bigint` 依存を排除してデフォルトゲートを安定させる。

範囲(MVP):
- `crates/kscr_unsafe_bigint` を追加（`num-bigint` 依存をここに閉じ込める）
- `--features unsafe_bigint` は `dep:kscr_unsafe_bigint` を有効化する
- `src/ir.rs` から `num_bigint::...` 参照を削除し、subcrate API 経由にする

Status: 完了（MVP）
- [x] subcrate 追加
- [x] feature配線
- [x] 既存テストが `--features unsafe_bigint` で通る

## Notes
- 以後「P5を実装」と言われたら **このファイルのP5** を実装する。
- 新しい優先度が必要になったら、このファイルを更新してから着手する。
