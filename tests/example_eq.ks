module EqExample where
  export main
  
  data Color = Red | Green | Blue deriving (Eq, Show)
  
  main = do
    stdoutWrite (show (Red == Red))
    stdoutWrite "\n"
    stdoutWrite (show (Red == Blue))
    stdoutWrite "\n"
