module Main where
  import Prelude

  -- Define a class with an infix operator method
  class Approx a where
    (=~=) :: a -> a -> Bool

  -- Instance that uses infix syntax for method definition
  instance Approx Integer where
    a =~= b = (a + 1) == (b + 1)

  -- Test data type with constructor patterns
  data Tree a = Leaf a | Node (Tree a) (Tree a)

  -- Infix operator that uses constructor patterns on both sides
  Leaf x ! Leaf y = Node (Leaf x) (Leaf y)
  Leaf x ! Node l r = Node (Leaf x) (Node l r)
  Node l r ! Leaf y = Node (Node l r) (Leaf y)
  Node l1 r1 ! Node l2 r2 = Node (Node l1 r1) (Node l2 r2)

  main = do
    putStrLn (toString (3 =~= 3))  -- True
    putStrLn (toString (3 =~= 5))  -- False
    -- Test constructor patterns in infix operators
    let tree1 = Leaf 1
    let tree2 = Leaf 2
    let combined = tree1 ! tree2
    putStrLn "Constructor pattern infix test passed"
