module Data.Either where
  export either, isLeft, isRight, fromLeft, fromRight

  import Prelude

  either f g e = case e of
    Left x -> f x
    Right y -> g y

  isLeft e = case e of
    Left _ -> True
    Right _ -> False

  isRight e = case e of
    Left _ -> False
    Right _ -> True

  fromLeft d e = case e of
    Left x -> x
    Right _ -> d

  fromRight d e = case e of
    Left _ -> d
    Right y -> y
