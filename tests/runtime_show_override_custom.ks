module Main where
  import Prelude

  data Box = Box Integer deriving (Eq, Show)

  instance Show Box where
    show = \_ -> "CUSTOM"

  main = print (Box 1)
