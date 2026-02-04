module Main where
  import Prelude

  data Person = Person [Char] Integer deriving (Eq, Show)

  main = do
    stdoutWrite (show (Person ['A','l','i','c','e'] 30))
    stdoutWrite "\n"

    if Person ['A','l','i','c','e'] 30 == Person ['A','l','i','c','e'] 30 && not (Person ['A','l','i','c','e'] 30 == Person ['B','o','b'] 20) then stdoutWrite "EQ_OK\n" else stdoutWrite "EQ_NG\n"
