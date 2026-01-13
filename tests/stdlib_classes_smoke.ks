module Main where
  import Prelude

  main = do
    stdoutWrite (show (fmap (\x -> x + 1) (Just 1)))
    stdoutWrite (show ((Just (\x -> x + 2)) <*> (Just 3)))
    stdoutWrite (show (1 <> 2))
    stdoutWrite (show ((invert 3) <> 10))
    IO ()
