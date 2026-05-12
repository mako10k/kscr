module Prelude.Monoid where
  export Monoid(..), mempty, mappend, mconcat

  import Prelude.Semigroup

  infixr 60 mappend

  class Semigroup a => Monoid a where
    mempty :: a
    mappend :: a -> a -> a
    mconcat :: [a] -> a
    minimal mempty

    mappend = (<>)
    mconcat xs = case xs of
      [] -> mempty
      y:ys -> mappend y (mconcat ys)
