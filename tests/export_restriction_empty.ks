-- Test: No explicit export - everything is accessible by default
module Main where
  import qualified ModuleEmpty as E

  main = putStrLn (show E.secret)  -- OK: secret is exported by default
