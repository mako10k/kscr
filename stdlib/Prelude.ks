module Prelude where
  export print, readLine

  print = stdoutWrite

  readLine = stdinReadLine
