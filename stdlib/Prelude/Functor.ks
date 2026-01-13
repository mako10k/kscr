module Prelude.Functor where
  export Functor(..), fmap, (<$>)

  infixl 40 <$>

  class Functor f where
    fmap :: (a -> b) -> f a -> f b

  (<$>) = fmap
