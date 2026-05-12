module Main where
  import Prelude

  data Box = Box Integer

  instance Eq Box where
    a == b = True

  main = do
    if Box 1 == Box 2 then stdoutWrite "EQ_OK\n" else stdoutWrite "EQ_NG\n"
