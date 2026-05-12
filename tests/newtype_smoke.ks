module Main where
  import Prelude

  newtype Age = Age Integer

  fromAge (Age n) = n

  value = fromAge (Age 42)
