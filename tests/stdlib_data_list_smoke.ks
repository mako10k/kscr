module Main where
  import Prelude as P

  main :: IO Unit
  main = do
    P.putStrLn (P.show (P.length (P.take 3 (P.enumFrom 1))))
    P.putStrLn (P.show (P.reverse [1,2,3]))
    P.putStrLn (P.show (P.head [10,20]))
