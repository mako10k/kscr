p = case v of
  () -> 0
  (a, b) -> a
  [x, y] -> x
  {a: x, b: y} -> x
  Just x -> x
