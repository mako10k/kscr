module Prelude where
  export String, print, readLine, getLine, putStr, putStrLn, error, Functor(..), fmap, (<$>), Applicative(..), pure, (<*>), (<*), (*>), Monad(..), (>>=), (>>), return, (=<<), Semigroup(..), (<>), Monoid(..), mempty, mappend, mconcat, Group(..), invert, (<->), Ring(..), zero, one, add, mul, neg, sub, (+^), (-^), (*^), negate, Field(..), inv, divide, (/^), recip, Integral(..), div, mod, quot, rem, Rational(..), numerator, denominator, toPair, Enum(..), enumFrom, enumFromThen, enumFromTo, enumFromThenTo, id, const, map, filter, concat, append, Maybe(..), Either(..), maybe, fromMaybe, isJust, isNothing, maybeToList, listToMaybe, mapMaybe, catMaybes

  import Prelude.Functor
  import Prelude.Applicative
  import Prelude.Monad
  import Prelude.Semigroup
  import Prelude.Monoid
  import Prelude.Group
  import Prelude.Ring
  import Prelude.Field
  import Prelude.Integral
  import Prelude.Rational

  -- Haskell-like alias: String ~ [Char]
  type String = [Char]

  class Enum a where
    enumFrom :: a -> [a]
    enumFromThen :: a -> a -> [a]
    enumFromTo :: a -> a -> [a]
    enumFromThenTo :: a -> a -> a -> [a]

  instance Enum Integer where
    enumFrom a = a : enumFrom (a + 1)

    enumFromThen a b = let step = b - a in a : enumFromThen (a + step) (a + step + step)

    enumFromTo a b = if a > b then [] else a : enumFromTo (a + 1) b

    enumFromThenTo a b c = let step = b - a in if step > 0 then (if a > c then [] else a : enumFromThenTo (a + step) (a + step + step) c) else (if a < c then [] else a : enumFromThenTo (a + step) (a + step + step) c)

  instance Functor IO where
    fmap f ma = __ioBind ma (\a -> IO (f a))

  instance Applicative IO where
    pure = IO
    mf <*> ma = __ioBind mf (\f -> __ioBind ma (\a -> IO (f a)))

  instance Monad IO where
    ma >>= f = __ioBind ma f
    ma >> mb = __ioThen ma mb

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

  data Maybe a = Nothing | Just a deriving Show

  instance Functor Maybe where
    fmap f m = case m of
      Nothing -> Nothing
      Just x -> Just (f x)

  instance Applicative Maybe where
    pure x = Just x
    mf <*> mx = case mf of
      Nothing -> Nothing
      Just f -> fmap f mx

  instance Monad Maybe where
    ma >>= f = case ma of
      Nothing -> Nothing
      Just x -> f x

  data Either a b = Left a | Right b deriving Show

  instance Semigroup Integer where
    a <> b = a + b

  instance Monoid Integer where
    mempty = 0

  instance Group Integer where
    invert x = 0 - x

  instance Ring Integer where
    zero = 0
    one = 1
    add a b = a + b
    mul a b = a * b
    neg a = 0 - a

  instance Integral Integer where
    div a b = __divInt a b
    mod a b = __modInt a b
    quot a b = __quotInt a b
    rem a b = __remInt a b

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
