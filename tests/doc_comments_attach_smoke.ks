-- | Doc for foo.
foo = 1

-- | Doc for Bar.
data Bar = Bar

{-| Block doc for Baz.
Second line.
-}
type Baz = Int

-- | Doc for Qux.
class Qux a where
  qux :: a -> Int

