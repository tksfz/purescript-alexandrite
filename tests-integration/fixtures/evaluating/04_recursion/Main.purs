module Main where

-- Use our FFI add/sub/eq for recursion
foreign import add :: Int -> Int -> Int
foreign import sub :: Int -> Int -> Int
foreign import eq :: Int -> Int -> Boolean

fib = \n -> 
  if eq n 0 then 0
  else if eq n 1 then 1
  else add (fib (sub n 1)) (fib (sub n 2))

test = fib 5
