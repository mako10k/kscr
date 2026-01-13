module Prelude.Group where
  export Group(..), invert, gsub, (<->)

  import Prelude.Monoid

  infixl 60 <->

  class Monoid a => Group a where
    invert :: a -> a

  gsub x y = mappend x (invert y)
  (<->) = gsub
