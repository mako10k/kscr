module TestDataTypes where

  -- Test data types for import spec tests
  data Color = Red | Green | Blue

  data Result a b = Ok a | Err b

  -- Some simple functions
  foo = 42
  bar = "hello"
  baz = True

