module Main where
  import Prelude
  import TestOps hiding (hidden)

  main :: IO ()
  main = print (hidden 1)  -- hidden is hidden, should fail
