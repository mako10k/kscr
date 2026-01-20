module Main where
	import Prelude

	-- Force a type mismatch using a type alias.
	-- Use a local alias that expands to the stdlib alias `String`.
	type Text = String

	bad = True :: Text
