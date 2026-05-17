module Main where

class Show a where
  show :: a -> String

instance showBool :: Show Boolean where
  show = \b -> if b then "true" else "false"

test = show true
