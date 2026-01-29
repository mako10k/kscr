module Main where
  import Prelude
  import TestOps (triple)

  main :: IO ()
  main = print (triple 3)
