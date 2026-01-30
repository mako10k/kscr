module Test where


  class C c where
    (!) :: c -> c -> c

  data D = D Integer

  instance C D where
    D a ! D b = D (a + b)

