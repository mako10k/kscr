module Main where
  main = do
    stdoutWrite (intToString (1 + 2))
    stdoutWrite "\n"
    stdoutWrite (boolToString (1 == 1))
    stdoutWrite "\n"
    IO ()
