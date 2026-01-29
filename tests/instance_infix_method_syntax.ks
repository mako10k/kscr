module Main where
  import Prelude

  -- Define a class with an infix operator method
  class Approx a where
    (=~=) :: a -> a -> Bool

  -- Instance that uses infix syntax for method definition
  instance Approx Integer where
    a =~= b = (a + 1) == (b + 1)

  main = do
    putStrLn (toString (3 =~= 3))  -- True
    putStrLn (toString (3 =~= 5))  -- False
