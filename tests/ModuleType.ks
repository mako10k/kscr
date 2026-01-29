-- Module with type export restriction: only constructor A is exported
module ModuleType (MyType(A)) where
  data MyType = A | B
