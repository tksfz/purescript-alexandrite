module Main where

class Eq a where
  eq :: a -> a -> Boolean

instance eqInt :: Eq Int where
  eq _ _ = true

test = eq 1 2
