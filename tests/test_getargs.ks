module Main where
  import Prelude

  main = do
    args <- getArgs
    putStrLn (toString (map (\arg -> arg) args))
