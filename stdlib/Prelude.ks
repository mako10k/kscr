module Prelude where
  export print, readLine, id, map, filter, concat, append

  print = stdoutWrite

  readLine = stdinReadLine

  id = \x -> x

  map = \f -> \xs -> concatMap (\x -> [f x]) xs

  filter = \p -> \xs -> concatMap (\x -> if p x then [x] else []) xs

  concat = \xss -> concatMap (\xs -> xs) xss

  append = \a -> \b -> a ++ b
