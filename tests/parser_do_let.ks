main = do
  let x = "hello"
  y <- return x
  putStrLn y

main2 = do
  let x = "hello" in
  y <- return x
  putStrLn y
