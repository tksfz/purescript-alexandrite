module Main where

import Effect (Effect)
import Data.Unit (Unit)

foreign import log :: String -> Effect Unit

test = log "Hello from PureScript interpreter!"
