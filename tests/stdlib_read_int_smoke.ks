module Main where
  import Prelude
  import Prelude.ReadClass

  main :: IO Unit
  main = do
    putStrLn (show (readMaybeInt "0"))
    putStrLn (show (readMaybeInt "  -42"))
    putStrLn (show (readMaybeInt "12x"))
    IO ()
