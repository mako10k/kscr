module Main where
  import Prelude

  -- Test that Num typeclass works with Integer instance
  -- The (+) and (*) operators should resolve via the Num class
  main = do
    putStrLn (toString (1 + 2))  -- 3
    putStrLn (toString (2 * 3))  -- 6
    putStrLn (toString (10 + 20 * 2))  -- 50
    putStrLn "Num typeclass test passed"
