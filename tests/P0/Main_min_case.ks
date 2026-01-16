module Main where
  import Prelude
  import Model as M
  import Logic as L

  m1 = M.mkOpt True 1

  main = case (L.eqOne m1) of
    True -> IO ()
    False -> IO ()
