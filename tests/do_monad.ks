module Main where
  import Prelude

  data Id a = Id a

  instance Functor Id where
    fmap f ma = case ma of
      Id x -> Id (f x)

  instance Applicative Id where
    pure x = Id x
    mf <*> mx = case mf of
      Id f -> fmap f mx

  instance Monad Id where
    ma >>= f = case ma of
      Id x -> f x

  x = do
    a <- Id 1
    b <- Id 2
    return (a + b)

  main = IO ()
