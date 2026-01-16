module Data.Maybe where
  export Maybe(..), maybe, fromMaybe, isJust, isNothing, maybeToList, listToMaybe, mapMaybe, catMaybes

  -- No Prelude import here to avoid name conflicts with Prelude's `maybe`.

  data Maybe a = Nothing | Just a

  id x = x

  concatMap f xs = case xs of
    [] -> []
    x:xt -> f x ++ concatMap f xt

  maybe d f m = case m of
    Nothing -> d
    Just x -> f x

  fromMaybe d m = maybe d id m

  isJust m = case m of
    Nothing -> False
    Just _ -> True

  isNothing m = case m of
    Nothing -> True
    Just _ -> False

  maybeToList m = case m of
    Nothing -> []
    Just x -> [x]

  listToMaybe xs = case xs of
    [] -> Nothing
    x:xt -> Just x

  mapMaybe f xs = concatMap (\x -> case f x of
    Nothing -> []
    Just y -> [y]
  ) xs

  catMaybes xs = mapMaybe id xs
