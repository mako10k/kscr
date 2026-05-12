module Main where
  import Prelude
  import Prelude.ReadClass

  emitRead :: Maybe Integer -> IO Unit
  emitRead m = case m of
    Nothing -> putStrLn "Nothing"
    Just n -> print n

  main :: IO Unit
  main = do
    emitRead (readMaybeInt "0")
    emitRead (readMaybeInt "  -42")
    emitRead (readMaybeInt "12x")
    IO ()
