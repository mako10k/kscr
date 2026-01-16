module Main where
  import Prelude
  import Model as M
  import Logic as L

  m1 = M.mkOpt True 1

  main = do
    stdoutWrite (show (L.eqOne m1))
    IO ()
