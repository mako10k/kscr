module Main where
  import Prelude

  printLines = \xs -> case xs of
    [] -> putStrLn "done"
    x:xt -> do
      putStrLn x
      printLines xt

  main = do
    printLines ["a", "b"]
