module Prelude.Monad where
  export Monad(..), (>>=), (>>), return

  infixl 10 >>=, >>

  class Monad m where
    (>>=) :: m a -> (a -> m b) -> m b
    (>>) :: m a -> m b -> m b
    return :: a -> m a

    ma >> mb = ma >>= (\_ -> mb)
