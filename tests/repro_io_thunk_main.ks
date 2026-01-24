module Main where
  import Prelude

  printLines = \xs -> case xs of
    [] -> putStrLn ""
    x:xt -> do
      putStrLn x
      printLines xt

  main = do
    printLines ["a", "b"]
