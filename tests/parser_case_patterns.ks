p = case v of
  () -> 0
  (a, b) -> a
  [x, y] -> x
  {a: x, b: y} -> x
  Just x -> x
  _ | if True then True else False -> 0
  0 | 1 -> 0
