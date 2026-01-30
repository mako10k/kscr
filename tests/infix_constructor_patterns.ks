module Main where
  import Prelude

  -- Test data types
  data Pair a b = P a b
  data Option a = None | Some a

  -- Infix operator with constructor patterns
  P x1 y1 ++ P x2 y2 = P (x1, x2) (y1, y2)

  -- Infix operator with mixed patterns
  Some a !! Some b = Some (a, b)
  None !! _ = None
  _ !! None = None

  -- Infix operator with nested constructor patterns
  P (Some x) _ >< P (Some y) _ = x == y
  P None _ >< P None _ = True
  _ >< _ = False

  -- Pattern binding (should still work)
  testPatternBinding :: [a] -> Bool
  testPatternBinding xs = case xs of
    [] -> True
    x:rest -> False

  main = do
    let p1 = P 1 2
    let p2 = P 3 4
    let result = p1 ++ p2
    
    let o1 = Some 5
    let o2 = Some 6
    let result2 = o1 !! o2
    
    let p3 = P (Some 10) 20
    let p4 = P (Some 10) 30
    let equal = p3 >< p4
    
    putStrLn "All constructor pattern infix tests passed"
