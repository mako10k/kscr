module Main where

	-- This file exists to ensure generic list Eq handles both String aliases and
	-- ordinary element-wise list equality.
	-- It should typecheck successfully.

	sameString :: String -> String -> Bool
	sameString left right = left == right

	sameCharList :: [Char] -> [Char] -> Bool
	sameCharList left right = left == right

	sameBoolList :: [Bool] -> [Bool] -> Bool
	sameBoolList left right = left == right

	main = print "ok"
