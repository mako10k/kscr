-- Module with export restrictions: only publicFunc is exported
module ModuleBasic (publicFunc) where
  publicFunc x = x + 1
  privateFunc x = x * 2
