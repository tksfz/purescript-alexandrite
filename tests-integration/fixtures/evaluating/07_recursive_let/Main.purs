module Main where

foreign import add :: Int -> Int -> Int
foreign import sub :: Int -> Int -> Int
foreign import eq :: Int -> Int -> Boolean

test =
  let
    sum = \n -> if eq n 0 then 0 else add n (sum (sub n 1))
  in
    sum 5
