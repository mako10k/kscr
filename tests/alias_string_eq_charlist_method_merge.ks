module Main where

	-- This file exists to ensure `String` and `[Char]` are treated as the same
	-- type when merging stdlib/typeclass method signatures.
	-- It should typecheck successfully.

	main = print "ok"
