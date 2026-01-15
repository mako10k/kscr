module Logic where
  import Prelude
  import Model as M
  export normalize, eqOne

  normalize o = case o of
    None -> 0
    Some n -> n + 1

  eqOne o = o == Some 1
