module Main where
  import Prelude as P

  main :: IO Unit
  main = case (P.Just 1) of
    P.Nothing -> stdoutWrite "n"
    P.Just _ -> stdoutWrite "j"
