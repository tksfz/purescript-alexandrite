module Main where

foreign import mul :: Int -> Int -> Int
foreign import div :: Int -> Int -> Int
foreign import mod :: Int -> Int -> Int
foreign import lt :: Int -> Int -> Boolean
foreign import gt :: Int -> Int -> Boolean
foreign import append :: String -> String -> String
foreign import length :: String -> Int

test = {
  mul: mul 6 7,
  div: div 100 2,
  mod: mod 10 3,
  lt: lt 5 10,
  gt: gt 10 5,
  append: append "Hello, " "World!",
  length: length "PureScript"
}
