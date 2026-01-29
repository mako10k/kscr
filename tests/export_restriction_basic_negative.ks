-- Negative test: attempting to use non-exported function should fail
module Main where
  import qualified ModuleBasic as M

  main = do
    putStrLn (show (M.privateFunc 5))  -- This should be a compile error
