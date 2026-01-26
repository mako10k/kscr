module Data.Maybe where
  export Maybe(..), maybe, fromMaybe, isJust, isNothing, maybeToList, listToMaybe, mapMaybe, catMaybes

  import Prelude

  -- Single-source of truth: `data Maybe` lives in Prelude.
  -- This alias enables qualified ctor re-export in the compiler:
  -- `import qualified Data.Maybe as M; M.Just / M.Nothing`.
  type Maybe a = Prelude.Maybe a

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
