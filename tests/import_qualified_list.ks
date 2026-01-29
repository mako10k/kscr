module Main where
  import Prelude
  import qualified TestMath as M (add, mul)

  main :: IO ()
  main = print (M.mul 2 5)
