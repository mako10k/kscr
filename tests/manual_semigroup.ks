module ManualSemigroup (main) where
  import Prelude
  
  data Pair = Pair Integer Integer
  
  instance Semigroup Pair where
    (<>) = \x y -> case (x, y) of
      (Pair a1 a2, Pair b1 b2) -> Pair (a1 <> b1) (a2 <> b2)
  
  p1 = Pair 1 2
  p2 = Pair 3 4
  p3 = p1 <> p2
  
  main = case p3 of
    Pair x y -> do
      stdoutWrite (show x)
      stdoutWrite " "
      stdoutWrite (show y)
      stdoutWrite "\n"
