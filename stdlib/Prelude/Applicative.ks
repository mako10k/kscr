module Prelude.Applicative where
  export Applicative(..), pure, (<*>), (*>), (<*)

  import Prelude.Functor

  infixl 40 <*>, <*, *>

  class Functor f => Applicative f where
    pure :: a -> f a
    (<*>) :: f (a -> b) -> f a -> f b
    (*>) :: f a -> f b -> f b
    (<*) :: f a -> f b -> f a
    minimal pure, (<*>)

    fa *> fb = (\_ -> \b -> b) <$> fa <*> fb
    fa <* fb = (\a -> \_ -> a) <$> fa <*> fb
