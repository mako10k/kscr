module Prelude.Num where
  export Num(..), (+), (*)

  infixl 60 +
  infixl 70 *

  class Num n where
    (+) :: n -> n -> n
    (*) :: n -> n -> n

  instance Num Integer where
    (+) = __builtin_Integer_add
    (*) = __builtin_Integer_mul
