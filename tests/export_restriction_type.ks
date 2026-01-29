-- Test: Type export restriction - only exported constructor is accessible
module Main where
  import qualified ModuleType as T

  main = do
    let x = T.A
    print "OK"
