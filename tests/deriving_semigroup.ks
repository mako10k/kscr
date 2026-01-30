module DerivingSemigroup (main) where
  import Prelude.Semigroup
  
  -- Test 1: Simple product type with Semigroup
  data Pair a = Pair a a deriving Semigroup
  
  -- Test 2: Combining pairs
  p1 = Pair 1 2
  p2 = Pair 3 4
  p3 = p1 <> p2
  
  -- Test 3: Single field
  data Wrapper a = Wrapper a deriving Semigroup
  
  w1 = Wrapper 10
  w2 = Wrapper 20
  w3 = w1 <> w2
  
  main = do
    case p3 of
      Pair x y -> do
        stdoutWrite (show x)
        stdoutWrite " "
        stdoutWrite (show y)
        stdoutWrite "\n"
    
    case w3 of
      Wrapper x -> do
        stdoutWrite (show x)
        stdoutWrite "\n"
