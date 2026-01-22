-- Ensures qualified class refs are accepted in predicates and instance heads.

module Main where

import qualified Prelude as P

-- predicate: allow `P.Show Integer`
f : P.Show Integer => Integer
f x = x

-- instance head: allow `instance P.Show Integer where`
instance P.Show Integer where
  show x = "ok"
