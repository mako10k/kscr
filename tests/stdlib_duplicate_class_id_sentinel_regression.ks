module Main where
  import Prelude
  import Prelude.Rational

  -- This is a smoke/regression module.
  -- It exists to exercise stdlib scanning + typeclass env loading after ModuleId(0) sentinel reservation.

  main :: IO Unit
  main = do
    putStrLn "ok"
