module Main where

-- Use our FFI add/sub for recursion
foreign import add :: Int -> Int -> Int
foreign import sub :: Int -> Int -> Int

fib = \n -> 
  if eq n 0 then 0
  else if eq n 1 then 1
  else add (fib (sub n 1)) (fib (sub n 2))

-- dummy eq for now as it's hardwired in our FFI
eq = \a -> \b -> true -- This won't work for recursion logic, let's add a proper eq FFI

test = fib 5
