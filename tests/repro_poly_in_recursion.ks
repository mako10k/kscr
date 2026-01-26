module Main where
  import Prelude

  printLines [] = return ()
  printLines (x:xt) = do
    putStrLn x
    printLines xt

  -- Use it at a concrete type.
  main = do
    printLines ["a", "b"]
