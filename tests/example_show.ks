module ShowExample where
  export main
  
  data Person = Person String Integer deriving Show
  
  main = do
    stdoutWrite (show (Person "Alice" 30))
    stdoutWrite "\n"
