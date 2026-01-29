-- Test: Multiple modules with different export restrictions
module Main where
  import qualified ModuleMultiA as MA
  import qualified ModuleMultiB as MB

  main = do
    putStrLn (show (MA.funcA 5))  -- OK: 15
    putStrLn (show (MB.funcB 5))  -- OK: 10
    let b1 = MB.B1
    let b2 = MB.B2
    putStrLn "All exports work correctly"
