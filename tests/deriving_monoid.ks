module DerivingMonoid (main) where
  import Prelude.Monoid
  
  -- Test 1: Simple product type with Monoid
  data Pair a = Pair a a deriving Monoid
  
  -- Test 2: mempty and mappend
  p1 = Pair 1 2
  p2 = mempty :: Pair Int
  p3 = mappend p1 p2
  
  -- Test 3: Single field
  data Wrapper a = Wrapper a deriving Monoid
  
  w1 = Wrapper 10
  w2 = mempty :: Wrapper Int
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
