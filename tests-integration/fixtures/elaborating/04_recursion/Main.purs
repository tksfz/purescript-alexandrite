module Main where

data Maybe a = Nothing | Just a

foreign import add :: Int -> Int -> Int
foreign import sub :: Int -> Int -> Int
foreign import eq :: Int -> Int -> Boolean

fib = \n -> 
  if eq n 0 then 0
  else if eq n 1 then 1
  else add (fib (sub n 1)) (fib (sub n 2))

-- Test ADT construction and matching
isJust = \m -> case m of
  Just _ -> true
  Nothing -> false

test = {
  res: fib 5,
  check: isJust (Just 1)
}
