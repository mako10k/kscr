# Prelude Coding Style

Goal: keep `stdlib/Prelude.ks` close to straightforward Haskell notation unless the current language or runtime requires a deviation.

Rules:

- Prefer the most direct Haskell surface syntax that the parser supports.
- For binary operators, prefer bare operator signatures and infix equations.
- Prefer `a == b = ...` over prefix wrappers such as `(==) a b = ...`.
- Prefer backtick infix calls for named binary operations when that is the clearest spelling.
- Eta-reduce simple wrappers when readability improves and behavior stays obvious.
- Keep comments short and only document intentional deviations from Haskell notation or semantics.
- When a Haskell-style spelling is not supported yet, add parser or typechecker support before introducing Prelude-only workarounds.