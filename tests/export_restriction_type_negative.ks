-- Negative test: attempting to use non-exported constructor should fail
module Main where
  import qualified ModuleType as T

  main = do
    let y = T.B  -- This should be a compile error
    print "Should not reach here"
