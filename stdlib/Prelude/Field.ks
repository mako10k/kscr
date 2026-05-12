module Prelude.Field where
  export Field(..), inv, divide, (/^), recip

  import Prelude.Ring

  infixl 70 /^

  class Ring a => Field a where
    inv :: a -> a
    minimal inv

    divide :: a -> a -> a
    divide x y = mul x (inv y)

  (/^) x y = divide x y

  recip = inv
