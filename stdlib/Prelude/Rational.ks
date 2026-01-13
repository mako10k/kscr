module Prelude.Rational where
  export Rational(..), numerator, denominator, toPair

  import Prelude.Ring
  import Prelude.Field
  import Prelude.Integral

  data Rational = Rat Integer Integer deriving (Eq, Show)

  numerator r = case r of
    Rat n _ -> n

  denominator r = case r of
    Rat _ d -> d

  toPair r = case r of
    Rat n d -> (n, d)

  instance Ring Rational where
    zero = Rat 0 1
    one = Rat 1 1

    add x y = case (x, y) of
      (Rat a b, Rat c d) -> if b * d == 0 then error "Rational: division by zero" else let num = a * d + c * b; den = b * d; n1 = if den < 0 then 0 - num else num; d1 = if den < 0 then 0 - den else den; absN = if n1 < 0 then 0 - n1 else n1; absD = if d1 < 0 then 0 - d1 else d1; g = if absD == 0 then absN else if absN == 0 then absD else let gcdLoop x1 y1 = if y1 == 0 then x1 else gcdLoop y1 (__modInt x1 y1) in gcdLoop absN absD in if n1 == 0 then Rat 0 1 else Rat (__quotInt n1 g) (__quotInt d1 g)

    mul x y = case (x, y) of
      (Rat a b, Rat c d) -> if b * d == 0 then error "Rational: division by zero" else let num = a * c; den = b * d; n1 = if den < 0 then 0 - num else num; d1 = if den < 0 then 0 - den else den; absN = if n1 < 0 then 0 - n1 else n1; absD = if d1 < 0 then 0 - d1 else d1; g = if absD == 0 then absN else if absN == 0 then absD else let gcdLoop x1 y1 = if y1 == 0 then x1 else gcdLoop y1 (__modInt x1 y1) in gcdLoop absN absD in if n1 == 0 then Rat 0 1 else Rat (__quotInt n1 g) (__quotInt d1 g)

    neg x = case x of
      Rat a b -> Rat (0 - a) b

  instance Field Rational where
    inv x = case x of
      Rat 0 _ -> error "Rational: reciprocal of zero"
      Rat a b -> if b == 0 then error "Rational: division by zero" else let n1 = if a < 0 then 0 - b else b; d1 = if a < 0 then 0 - a else a; absN = if n1 < 0 then 0 - n1 else n1; absD = if d1 < 0 then 0 - d1 else d1; g = if absD == 0 then absN else if absN == 0 then absD else let gcdLoop x1 y1 = if y1 == 0 then x1 else gcdLoop y1 (__modInt x1 y1) in gcdLoop absN absD in if b == 0 then Rat 0 1 else Rat (__quotInt n1 g) (__quotInt d1 g)
