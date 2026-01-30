-- Test file for infix dot and dollar operators

module InfixDotDollar where

  -- Define function composition (.) as an infix operator
  (.) f g x = f (g x)

  -- Define application operator ($)
  ($) f x = f x

  -- Helper functions for testing
  double :: Integer -> Integer
  double x = x * 2

  increment :: Integer -> Integer
  increment x = x + 1

  square :: Integer -> Integer
  square x = x * x

  -- Test function composition with spaces: f . g
  composedWithSpaces :: Integer -> Integer
  composedWithSpaces = double . increment . square

  -- Test application with $
  testDollar :: Integer
  testDollar = double $ increment $ square 3

  main :: IO ()
  main = do
    -- Test composition: composedWithSpaces 3 should be (3*3+1)*2 = 20
    putStrLn $ "composedWithSpaces 3 = " ++ show (composedWithSpaces 3)
    
    -- Test $: should be the same as composedWithSpaces 3
    putStrLn $ "testDollar = " ++ show testDollar
    
    -- Both should equal 20
    if composedWithSpaces 3 == 20 && testDollar == 20 then putStrLn "PASS" else putStrLn "FAIL"
