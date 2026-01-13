module Data.List where
  export List, null, singleton, head, tail, map, filter, append, concat, concatMap

  -- Haskell-like alias: List a ~ [a]
  type List a = [a]

  null = \xs -> case xs of
    [] -> True
    _:_ -> False

  singleton = \x -> [x]

  head = \xs -> case xs of
    x:_ -> x

  tail = \xs -> case xs of
    _:xt -> xt

  map = \f -> \xs -> concatMap (\x -> [f x]) xs

  filter = \p -> \xs -> concatMap (\x -> if p x then [x] else []) xs

  append = \a -> \b -> concatMap (\xs -> xs) [a, b]

  concat = \xss -> concatMap (\xs -> xs) xss
