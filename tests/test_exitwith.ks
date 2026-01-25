module Main where
  import Prelude

  main = do
    putStrLn "About to exit with code 42"
    _ <- exitWith 42
    putStrLn "This should not be printed"
