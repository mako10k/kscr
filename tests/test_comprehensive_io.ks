module Main where
  import Prelude

  -- Comprehensive test that demonstrates all new I/O APIs:
  -- getArgs, readFile, writeFile, exitWith

  main = do
    -- Get command-line arguments
    args <- getArgs
    putStrLn "Command-line arguments:"
    putStrLn (toString args)
    
    -- Write a file
    putStrLn "\nWriting to file..."
    writeFile "demo_output.txt" "This is a test file.\nIt has multiple lines.\n"
    
    -- Read the file back
    putStrLn "Reading from file..."
    content <- readFile "demo_output.txt"
    putStr "Content:\n"
    putStr content
    
    -- Demonstrate conditional exit
    case args of
      [] -> putStrLn "\nNo arguments provided. Exiting normally."
      ("exit":code:_) -> do
        putStrLn ("\nExit code argument found: " ++ code)
        exitWith 99
      _ -> putStrLn "\nArguments provided. Continuing normally."
