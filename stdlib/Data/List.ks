module Data.List where
  export List, Maybe(..), null, singleton, head, tail, map, filter, append, concat, concatMap, length, foldr, foldl, reverse, take, drop, takeWhile, dropWhile, any, all, elem, find, zip, unzip

  -- Haskell-like alias: List a ~ [a]
  type List a = [a]

  data Maybe a = Nothing | Just a

  null [] = True
  null _:_ = False

  singleton x = [x]

  head [] = error "head: empty list"
  head (x:_) = x

  tail [] = error "tail: empty list"
  tail (_:xs) = xs

  map f xs = concatMap (\x -> [f x]) xs

  filter p xs = concatMap (\x -> if p x then [x] else []) xs

  append [] ys = ys
  append (x:xs) ys = x : append xs ys

  concat xss = concatMap (\xs -> xs) xss

  length [] = 0
  length (_:xs) = 1 + length xs

  foldr f z [] = z
  foldr f z (x:xs) = f x (foldr f z xs)

  foldl f z [] = z
  foldl f z (x:xs) = foldl f (f z x) xs

  reverse xs = foldl (\acc -> \x -> x:acc) [] xs

  take n xs = if n == 0 then [] else case xs of
    [] -> []
    x:xt -> x : take (n - 1) xt

  drop n xs = if n == 0 then xs else case xs of
    [] -> []
    _:xt -> drop (n - 1) xt

  takeWhile p [] = []
  takeWhile p (x:xs) = if p x then x : takeWhile p xs else []

  dropWhile p [] = []
  dropWhile p (x:xs) = if p x then dropWhile p xs else x:xs

  any p [] = False
  any p (x:xs) = if p x then True else any p xs

  all p [] = True
  all p (x:xs) = if p x then all p xs else False

  elem x [] = False
  elem x (y:ys) = if x == y then True else elem x ys

  find p [] = Nothing
  find p (x:xs) = if p x then Just x else find p xs

  zip [] _ = []
  zip _ [] = []
  zip (x:xs) (y:ys) = (x, y) : zip xs ys

  unzip [] = ([], [])
  unzip ((x, y):xys) = case unzip xys of
    (xs, ys) -> (x:xs, y:ys)
