module Prelude.Monad where
  export Monad(..), (>>=), (>>), return, (=<<)

  import Prelude.Applicative

  infixl 10 >>=, >>, =<<

  class Applicative m => Monad m where
    (>>=) :: m a -> (a -> m b) -> m b
    (>>) :: m a -> m b -> m b
    return :: a -> m a
    minimal (>>=)

    return = pure

    ma >> mb = ma >>= (\_ -> mb)

  (=<<) f ma = ma >>= f
