module NumOperatorsBasic where
  import Prelude

  testAdd = 1 + 2
  testMul = 2 * 3
  
  main = do
    putStrLn (toString testAdd)
    putStrLn (toString testMul)
