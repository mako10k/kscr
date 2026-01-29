module Main where
  import Prelude
  import TestMath ()

  -- Define our own add
  add :: Integer -> Integer -> Integer
  add x y = x + y + 40

  main :: IO ()
  main = print (add 1 1)
