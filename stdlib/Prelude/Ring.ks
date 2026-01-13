module Prelude.Ring where
  export Ring(..), zero, one, add, mul, neg, sub, (+^), (-^), (*^), negate

  infixl 60 +^, -^
  infixl 70 *^

  class Ring a where
    zero :: a
    one :: a
    add :: a -> a -> a
    mul :: a -> a -> a
    neg :: a -> a

    sub :: a -> a -> a
    sub x y = add x (neg y)

  (+^) x y = add x y

  (*^) x y = mul x y

  (-^) x y = sub x y

  negate = neg
