module Main where
  import Prelude
  import Model as M
  import Logic as L

  m1 = M.mkOpt True 1
  m2 = M.mkOpt False 1

  main = do
    case (L.eqOne m1) of
      True -> stdoutWrite "ok\n"
      False -> throw "assert failed: eqOne"
    case (L.normalize m2) of
      0 -> stdoutWrite (show m2)
      _ -> throw "assert failed: normalize"
    IO ()
