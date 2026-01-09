module Prelude where
  export print, readLine, getLine, putStr, putStrLn, id, const, map, filter, concat, append, Maybe, Either, maybe, fromMaybe, isJust, isNothing, maybeToList, listToMaybe, mapMaybe, catMaybes

  print = stdoutWrite

  readLine = stdinReadLine

  getLine = readLine

  putStr = stdoutWrite

  putStrLn = \s -> stdoutWrite (s ++ "\n")

  id = \x -> x

  const = \x -> \_ -> x

  map = \f -> \xs -> concatMap (\x -> [f x]) xs

  filter = \p -> \xs -> concatMap (\x -> if p x then [x] else []) xs

  concat = \xss -> concatMap (\xs -> xs) xss

  append = \a -> \b -> a ++ b

  data Maybe a = Nothing | Just a

  data Either a b = Left a | Right b

  maybe = \d -> \f -> \m -> case m of
    Nothing -> d
    Just x -> f x

  fromMaybe = \d -> \m -> maybe d id m

  isJust = \m -> case m of
    Nothing -> False
    Just _ -> True

  isNothing = \m -> case m of
    Nothing -> True
    Just _ -> False

  maybeToList = \m -> case m of
    Nothing -> []
    Just x -> [x]

  listToMaybe = \xs -> case xs of
    [] -> Nothing
    x:xt -> Just x

  mapMaybe = \f -> \xs -> concatMap (\x -> case f x of
    Nothing -> []
    Just y -> [y]
  ) xs

  catMaybes = \xs -> mapMaybe id xs
