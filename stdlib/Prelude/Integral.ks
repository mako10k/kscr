module Prelude.Integral where
  export Integral(..), div, mod, quot, rem

  import Prelude.Ring

  class Ring a => Integral a where
    div :: a -> a -> a
    mod :: a -> a -> a
    quot :: a -> a -> a
    rem :: a -> a -> a
