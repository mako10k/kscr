-- Test: Basic export restriction - only exported function is accessible
import qualified ModuleBasic as M

main = do
  putStrLn (show (M.publicFunc 5))  -- OK, should output: 6
