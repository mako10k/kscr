module Main where
  import qualified Prelude as P

  main :: IO Unit
  main = case (P.Nothing) of
    P.Nothing -> stdoutWrite "n"
    P.Just _ -> stdoutWrite "j"
