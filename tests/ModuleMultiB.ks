-- Module B with export restrictions: export all constructors of TypeB
module ModuleMultiB (funcB, TypeB(..)) where
  funcB x = x * 2
  secretB = 200
  data TypeB = B1 | B2
