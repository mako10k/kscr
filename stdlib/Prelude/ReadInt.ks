module Prelude.ReadInt where
  export readIntMaybe

  import Prelude.Read

  digitToInt :: Char -> Maybe Integer
  digitToInt c = if c == '0' then Just 0 else
    if c == '1' then Just 1 else
    if c == '2' then Just 2 else
    if c == '3' then Just 3 else
    if c == '4' then Just 4 else
    if c == '5' then Just 5 else
    if c == '6' then Just 6 else
    if c == '7' then Just 7 else
    if c == '8' then Just 8 else
    if c == '9' then Just 9 else Nothing

  isDigit :: Char -> Bool
  isDigit c = case digitToInt c of
    Nothing -> False
    Just _ -> True

  parseNat :: Parser Integer
  parseNat = mapP digitsToNat (many1 (satisfy isDigit))

  digitsToNat :: [Char] -> Integer
  digitsToNat cs = foldDigits cs 0

  foldDigits :: [Char] -> Integer -> Integer
  foldDigits cs acc = case cs of
    [] -> acc
    c:ct -> case digitToInt c of
      Nothing -> acc
      Just d -> foldDigits ct (acc * 10 + d)

  parseSign :: Parser Integer
  parseSign = orP parseMinus (orP parsePlus (pureP 1))

  parseMinus :: Parser Integer
  parseMinus = mapP (\_ -> 0 - 1) (char '-')

  parsePlus :: Parser Integer
  parsePlus = mapP (\_ -> 1) (char '+')

  parseInt :: Parser Integer
  parseInt = bindP parseSign (\s -> bindP parseNat (\n -> pureP (s * n)))

  readIntMaybe :: String -> Maybe Integer
  readIntMaybe s = case runParser (token parseInt) s of
    Nothing -> Nothing
    Just (n, rest) -> case runParser whitespace rest of
      Nothing -> Nothing
      Just (_, r2) -> case r2 of
        [] -> Just n
        _ -> Nothing
