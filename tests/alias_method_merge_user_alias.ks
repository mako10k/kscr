module Main where

	-- Regression: method signature comparisons during ClassEnv merge must treat
	-- type aliases as equal to their expansions.
	--
	-- This is intentionally tiny: defining a local alias should not break
	-- typechecking when stdlib class env is merged.

	type Text = String

	main = print "ok"
