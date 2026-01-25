module Main where
  import Prelude

  main = do
    writeFile "test_output.txt" "Hello, World!\n"
    content <- readFile "test_output.txt"
    putStr content
