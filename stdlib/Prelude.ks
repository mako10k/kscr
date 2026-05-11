module Prelude where
  export String, print, readLine, getLine, putStr, putStrLn, getArgs, readFile, writeFile, exitWith, error, Functor(..), fmap, (<$>), Applicative(..), pure, (<*>), (<*), (*>), Monad(..), (>>=), (>>), return, (=<<), Semigroup(..), (<>), Monoid(..), mempty, mappend, mconcat, Group(..), invert, (<->), Ring(..), zero, one, add, mul, neg, sub, (+^), (-^), (*^), negate, Field(..), inv, divide, (/^), recip, Integral(..), div, mod, quot, rem, Eq(..), eq, (==), (/=), Show(..), show, toString, Rational(..), numerator, denominator, toPair, Num(..), (+), (*), Enum(..), enumFrom, enumFromThen, enumFromTo, enumFromThenTo, id, const, (.), ($), flip, (&), on, until, concatMap, map, filter, concat, append, length, foldr, foldl, reverse, take, drop, takeWhile, dropWhile, any, all, elem, find, zip, unzip, Maybe(..), Either(..), maybe, fromMaybe, isJust, isNothing, maybeToList, listToMaybe, mapMaybe, catMaybes

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
  import Prelude.Num

  -- Haskell-like alias: String ~ [Char]
  type String = [Char]

  class Enum a where
    enumFrom :: a -> [a]
    enumFromThen :: a -> a -> [a]
    enumFromTo :: a -> a -> [a]
    enumFromThenTo :: a -> a -> a -> [a]

  infix 40 ==
  infix 40 /=

  class Eq a where
    eq :: a -> a -> Bool

  (==) a b = eq a b

  -- Default Eq instances for primitive types.
  -- These are normal instances (not special-cased primitives), but their implementations
  -- delegate to the structural runtime builtin.
  instance Eq Integer where
    eq = __primEq

  instance Eq Char where
    eq = __primEq

  instance Eq Bool where
    eq = __primEq

  instance Eq Unit where
    eq = __primEq

  instance Eq a => Eq [a] where
    eq xs ys = case (xs, ys) of
      ([], []) -> True
      (x : xs1, y : ys1) -> if x == y then eq xs1 ys1 else False
      _ -> False

  (/=) = \a -> \b -> if a == b then False else True

  class Show a where
    show :: a -> [Char]

  -- Haskell-like alias: `toString` via Show.
  toString x = show x

  -- Default Show instances for primitive types.
  -- These are normal instances (not special-cased primitives), but their implementations
  -- delegate to stable primitives.
  instance Show Integer where
    show = intToString

  instance Show Bool where
    show = boolToString

  instance Show Char where
    show = __primShow

  instance Show Unit where
    show = __primShow

  instance Show Float64 where
    show = __primShow

  instance Show String where
    show = \s -> s

  -- Composite Show instances (polymorphic heads).
  -- Delegate to the structural runtime builtin for MVP.
  instance Show [a] where
    show = __primShow

  instance Show (a, b) where
    show = __primShow

  instance Show {a: a, b: b} where
    show = __primShow

  instance Enum Integer where
    enumFrom a = a : enumFrom (a + 1)

    enumFromThen a b = let step = b - a in a : enumFromThen (a + step) (a + step + step)

    enumFromTo a b = if a > b then [] else a : enumFromTo (a + 1) b

    enumFromThenTo a b c = let step = b - a in if step > 0 then (if a > c then [] else a : enumFromThenTo (a + step) (a + step + step) c) else (if a < c then [] else a : enumFromThenTo (a + step) (a + step + step) c)

  -- Note: list range sugar (`[a..b]`, `[a,b..c]`) currently desugars to `enumFromTo` /
  -- `enumFromThenTo` as if they were ordinary top-level functions. In this implementation,
  -- these are class methods only; keep the names reserved here but do not define wrappers.

  instance Functor IO where
    fmap f ma = __ioBind ma (\a -> IO (f a))

  instance Applicative IO where
    pure = IO
    mf <*> ma = __ioBind mf (\f -> __ioBind ma (\a -> IO (f a)))

  instance Monad IO where
    ma >>= f = __ioBind ma f
    ma >> mb = __ioThen ma mb

  -- Haskell-compatible: print via Show + newline.
  print = \x -> putStrLn (toString x)

  readLine = stdinReadLine

  getLine = readLine

  putStr = stdoutWrite

  putStrLn = \s -> stdoutWrite (s ++ "\n")

  id = \x -> x

  const = \x -> \_ -> x

  -- Function composition operator
  (.) f g x = f (g x)

  -- Function application operator
  ($) f x = f x

  -- Flip the first two arguments of a function
  flip f x y = f y x

  -- Reverse application operator
  (&) x f = f x

  -- Binary function on outputs: on f g x y = f (g x) (g y)
  on f g x y = f (g x) (g y)

  -- Apply function until predicate holds
  until = \p -> \f -> \x -> if p x then x else until p f (f x)

  concatMap = \f -> \xs -> case xs of
    [] -> []
    x:xt -> f x ++ concatMap f xt

  map = \f -> \xs -> concatMap (\x -> [f x]) xs

  filter = \p -> \xs -> concatMap (\x -> if p x then [x] else []) xs

  concat = \xss -> concatMap (\xs -> xs) xss

  append = \a -> \b -> a ++ b

  length = \xs -> case xs of
    [] -> 0
    _:xt -> 1 + length xt

  foldr = \f -> \z -> \xs -> case xs of
    [] -> z
    x:xt -> f x (foldr f z xt)

  foldl = \f -> \z -> \xs -> case xs of
    [] -> z
    x:xt -> foldl f (f z x) xt

  reverse = \xs -> foldl (\acc -> \x -> x:acc) [] xs

  take = \n -> \xs -> if n == 0 then [] else case xs of
    [] -> []
    x:xt -> x : take (n - 1) xt

  drop = \n -> \xs -> if n == 0 then xs else case xs of
    [] -> []
    _:xt -> drop (n - 1) xt

  takeWhile = \p -> \xs -> case xs of
    [] -> []
    x:xt -> if p x then x : takeWhile p xt else []

  dropWhile = \p -> \xs -> case xs of
    [] -> []
    x:xt -> if p x then dropWhile p xt else x:xt

  any = \p -> \xs -> case xs of
    [] -> False
    x:xt -> if p x then True else any p xt

  all = \p -> \xs -> case xs of
    [] -> True
    x:xt -> if p x then all p xt else False

  elem = \x -> \xs -> case xs of
    [] -> False
    y:yt -> if x == y then True else elem x yt

  find = \p -> \xs -> case xs of
    [] -> Nothing
    x:xt -> if p x then Just x else find p xt

  zip = \xs -> \ys -> case xs of
    [] -> []
    x:xt -> case ys of
      [] -> []
      y:yt -> (x, y) : zip xt yt

  unzip = \xys -> case xys of
    [] -> ([], [])
    (x, y):xt -> case unzip xt of
      (xs, ys) -> (x:xs, y:ys)

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
