module TestMath (add, sub, mul, pow) where
  add :: Integer -> Integer -> Integer
  add x y = x + y

  sub :: Integer -> Integer -> Integer
  sub x y = x - y

  mul :: Integer -> Integer -> Integer
  mul x y = x * y

  pow :: Integer -> Integer -> Integer
  pow x y = if y == 0 then 1 else x * pow x (y - 1)
