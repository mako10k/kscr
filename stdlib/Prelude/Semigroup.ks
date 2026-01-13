module Prelude.Semigroup where
  export Semigroup(..), (<>)

  infixr 60 <>

  class Semigroup a where
    (<>) :: a -> a -> a
