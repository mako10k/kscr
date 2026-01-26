module Main where
  import Prelude

  printLines [] = getLine
  printLines (x:xt) = do
    putStrLn x
    printLines xt

  main = do
    _ <- printLines ["a", "b"]
    putStrLn "done"
