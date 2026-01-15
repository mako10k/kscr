module Model where
  import Prelude
  export Opt(..), mkOpt, unwrapOr

  data Opt a = None | Some a deriving (Eq, Show)

  mkOpt b x = if b then Some x else None

  unwrapOr def o = case o of
    None -> def
    Some x -> x
