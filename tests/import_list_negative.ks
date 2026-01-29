module Main where
  import Prelude
  import TestMath (add, sub)

  main :: IO ()
  main = print (pow 2 3)  -- pow is not imported, should fail
