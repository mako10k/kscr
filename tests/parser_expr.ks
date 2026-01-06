-- lambda式
id = \x -> x
const = \x y -> x

-- if式
cond = if True then 1 else 0

-- 関数適用
applied = f x y

-- ネスト
nested = \x -> if x then f x else g x
