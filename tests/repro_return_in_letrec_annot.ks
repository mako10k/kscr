module Main where
  import Prelude

  f :: [String] -> IO Unit
  f [] = return ()
  f (x:xs) = do
    putStrLn x
    f xs

  main = do
    f ["a"]
