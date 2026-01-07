main = do
  (x, y) <- f 1
  z <- g x
  h x y z
