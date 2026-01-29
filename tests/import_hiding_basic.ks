module Main where
  import Prelude
  import TestMath hiding (mul)

  main :: IO ()
  main = print (sub 7 2)
