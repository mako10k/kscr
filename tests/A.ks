module A where

  class Inc a where
      inc :: a -> a

  instance Inc Integer where
      inc x = x + 1
