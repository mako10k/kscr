module DerivingSemigroupZero (main) where
  import Prelude.Semigroup
  
  -- Test with zero fields (should work without needing constraint resolution)
  data Unit = Unit deriving Semigroup
  
  u1 = Unit
  u2 = Unit
  u3 = u1 <> u2
  
  main = do
    case u3 of
      Unit -> stdoutWrite "Unit combined\n"
      _ -> stdoutWrite "Unreachable\n"
