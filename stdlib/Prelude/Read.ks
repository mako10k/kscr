module Prelude.Read where
  export Parser, runParser, pureP, failP, mapP, bindP, orP, satisfy, char, many, many1, whitespace, token, eof

  -- A tiny parser over String ([Char]).
  -- Keep this module Prelude-independent to avoid cyclic stdlib imports.

  type Parser a = String -> Maybe (a, String)

  runParser :: Parser a -> String -> Maybe (a, String)
  runParser p s = p s

  pureP :: a -> Parser a
  pureP a = \s -> Just (a, s)

  failP :: Parser a
  failP = \_ -> Nothing

  mapP :: (a -> b) -> Parser a -> Parser b
  mapP f p = \s -> case p s of
    Nothing -> Nothing
    Just (a, rest) -> Just (f a, rest)

  bindP :: Parser a -> (a -> Parser b) -> Parser b
  bindP p f = \s -> case p s of
    Nothing -> Nothing
    Just (a, rest) -> f a rest

  orP :: Parser a -> Parser a -> Parser a
  orP p q = \s -> case p s of
    Nothing -> q s
    ok -> ok

  satisfy :: (Char -> Bool) -> Parser Char
  satisfy pred = \s -> case s of
    [] -> Nothing
    c:cs -> if pred c then Just (c, cs) else Nothing

  char :: Char -> Parser Char
  char t = satisfy (\c -> c == t)

  many :: Parser a -> Parser [a]
  many p = orP (many1 p) (pureP [])

  many1 :: Parser a -> Parser [a]
  many1 p = bindP p (\x -> mapP (\xs -> x:xs) (many p))

  isSpace :: Char -> Bool
  isSpace c = c == ' ' || c == '\n' || c == '\t' || c == '\r'

  whitespace :: Parser Unit
  whitespace = mapP (\_ -> ()) (many (satisfy isSpace))

  token :: Parser a -> Parser a
  token p = bindP whitespace (\_ -> bindP p (\a -> bindP whitespace (\_ -> pureP a)))

  eof :: Parser Unit
  eof = \s -> case s of
    [] -> Just ((), [])
    _ -> Nothing
