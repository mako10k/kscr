module Main where
  import Prelude

  main :: IO Unit
  main = do
    -- Test: "" ++ ['a']
    putStrLn ("" ++ ['a'])
    
    -- Test: ['b'] ++ ""
    putStrLn (['b'] ++ "")
    
    -- Test: "x" ++ ['y']
    putStrLn ("x" ++ ['y'])
    
    -- Test: ['c'] ++ "d"
    putStrLn (['c'] ++ "d")
    
    -- Test: [] ++ ['e']
    putStrLn ([] ++ ['e'])
