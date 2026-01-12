module Main where
  import Prelude

  data Id a = Id a

  instance Monad Id where
    ma >>= f = case ma of
      Id x -> f x

    return = \x -> Id x

  x = do
    a <- Id 1
    b <- Id 2
    return (a + b)

  main = IO ()
