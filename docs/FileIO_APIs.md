# File I/O and Command-Line APIs

This document describes the minimal file I/O and command-line argument APIs added to enable self-hosting toolchain development.

## Overview

The following APIs are available in the `Prelude` module:

- `getArgs :: IO [[Char]]` - Get command-line arguments
- `readFile :: [Char] -> IO [Char]` - Read a file's contents
- `writeFile :: [Char] -> [Char] -> IO Unit` - Write contents to a file
- `exitWith :: Integer -> IO a` - Exit the program with a status code

## API Details

### getArgs

```haskell
getArgs :: IO [[Char]]
```

Returns all command-line arguments passed to the program, including the binary path and subcommand (if any).

**Example:**
```haskell
main = do
  args <- getArgs
  putStrLn (toString args)
```

Running `kscr run myfile.ks arg1 arg2` will output:
```
["kscr", "run", "myfile.ks", "arg1", "arg2"]
```

### readFile

```haskell
readFile :: [Char] -> IO [Char]
```

Reads the entire contents of a file as a string (character list). Returns an error if the file cannot be read.

**Example:**
```haskell
main = do
  content <- readFile "input.txt"
  putStr content
```

### writeFile

```haskell
writeFile :: [Char] -> [Char] -> IO Unit
```

Writes the given content to a file, creating it if it doesn't exist or overwriting it if it does. Returns an error if the file cannot be written.

**Example:**
```haskell
main = do
  writeFile "output.txt" "Hello, World!\n"
  putStrLn "File written successfully"
```

### exitWith

```haskell
exitWith :: Integer -> IO a
```

Exits the program immediately with the given status code. This function never returns, which is why it has the polymorphic return type `IO a`.

**Example:**
```haskell
main = do
  args <- getArgs
  case args of
    [] -> do
      putStrLn "Error: No arguments provided"
      exitWith 1
    _ -> putStrLn "Success"
```

## String Representation

All these APIs work with the Prelude type `String = [Char]` (character lists). The runtime automatically converts between:
- Runtime primitive strings (`Value::String`)
- Character lists (`Value::ListCons` of `Value::Char`)

This conversion is transparent to user code. You can pass string literals or character lists to these functions interchangeably.

## Error Handling

File operations (`readFile`, `writeFile`) will terminate the program with an error message if they fail. For example:

- `readFile` on a non-existent file: `"readFile: failed to read 'missing.txt': No such file or directory"`
- `writeFile` on a read-only file: `"writeFile: failed to write 'readonly.txt': Permission denied"`

## Use Cases

These APIs enable:

1. **Self-hosting compiler development**: Reading source files, writing compiled output
2. **Command-line tools**: Processing arguments, reading/writing files
3. **Build scripts**: Automating tasks with file I/O
4. **Test frameworks**: Reading test fixtures, writing test results

## Implementation Notes

- File paths are relative to the current working directory
- File I/O is blocking and synchronous
- `exitWith` uses `std::process::exit()` and immediately terminates the process
- Command-line arguments include the full invocation (binary path + subcommand + arguments)

## Examples

See the test files for complete examples:
- `tests/test_getargs.ks` - Command-line argument handling
- `tests/test_read_write_file.ks` - File I/O operations
- `tests/test_exitwith.ks` - Exit status codes
- `tests/test_comprehensive_io.ks` - All APIs working together
