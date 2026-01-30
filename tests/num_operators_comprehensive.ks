module NumOperatorsComprehensive where
  import Prelude

  -- Basic operators
  testAdd = 1 + 2
  testMul = 2 * 3
  
  -- Nested expressions
  testNested = (1 + 2) * (3 + 4)
  
  -- More complex
  testComplex = (5 + 10) * 2 + (3 * 4)
  
  main = do
    putStrLn (toString testAdd)
    putStrLn (toString testMul)
    putStrLn (toString testNested)
    putStrLn (toString testComplex)
