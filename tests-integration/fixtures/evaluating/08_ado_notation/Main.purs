module Main where

foreign import map :: forall a b. (a -> b) -> a -> b
foreign import apply :: forall a b. (a -> b) -> a -> b
foreign import pure :: forall a. a -> a

test = ado
  x <- pure 20
  y <- pure 22
  in x + y

infixl 6 add as +
foreign import add :: Int -> Int -> Int
