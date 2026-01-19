-- Regression: class method call should not require ground argument types.
-- This should typecheck with a constraint, not error during method-call rewriting.

canDiv n d = (n `mod` d) /= 0
