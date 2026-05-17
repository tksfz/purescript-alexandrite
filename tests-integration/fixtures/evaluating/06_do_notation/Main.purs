module Main where

foreign import bind :: forall a b. a -> (a -> b) -> b
foreign import discard :: forall a b. a -> (unit -> b) -> b
foreign import log :: String -> {} -> {}
foreign import pure :: forall a. a -> a

test = do
  log "Step 1"
  x <- pure 42
  log "Step 2"
  pure x
