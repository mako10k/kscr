module DerivingNewline (main) where
  -- Test parser: newlines around deriving clause
  
  data Foo = Foo Int
    deriving Show
  
  data Bar = Bar String String
    deriving (Eq, Show)
  
  data Baz = Baz Bool
    deriving
      (Eq,
       Show)
  
  main = do
    stdoutWrite (show (Foo 42))
    stdoutWrite "\n"
    stdoutWrite (show (Bar "a" "b" == Bar "a" "b"))
    stdoutWrite "\n"
    stdoutWrite (show (Baz True == Baz False))
    stdoutWrite "\n"
