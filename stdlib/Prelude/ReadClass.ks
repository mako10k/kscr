module Prelude.ReadClass where
  export ReadFromString(..), readMaybeInt

  import Prelude.Read
  import Prelude.ReadInt

  -- Minimal ReadFromString class. Keep it separate from Prelude to avoid cyclic imports.
  class ReadFromString a where
    readMaybe :: [Char] -> Maybe a

  instance ReadFromString Integer where
    readMaybe = readIntMaybe

  -- Convenient named helper for early callers.
  readMaybeInt :: String -> Maybe Integer
  readMaybeInt = readIntMaybe
