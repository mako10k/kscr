data Maybe a = Nothing | Just a deriving Show
type String = [Char]

data Pair a b = a :*: b deriving Show
p = 1 :*: 2
q = (:*:) 1 2
r = (1 :*:) 2
fstPair (a :*: b) = a

x = 1
flag = True
msg = "hello"
