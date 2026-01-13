module Main where
  import Prelude
  import Prelude.Rational

  main = do
    stdoutWrite (show (add (Rat 1 2) (Rat 1 3)))
    stdoutWrite (show (mul (Rat 1 2) (Rat 1 3)))
    stdoutWrite (show (divide (Rat 1 2) (Rat 1 3)))
    stdoutWrite (show (__quotInt 7 2))
    IO ()
