module Main where
  import Prelude

  data Person = Person String Integer deriving (Eq, Show)

  main = do
    stdoutWrite (show (Person "Alice" 30))
    stdoutWrite "\n"

    if Person "Alice" 30 == Person "Alice" 30
      then stdoutWrite "EQ_OK\n"
      else stdoutWrite "EQ_NG\n"
