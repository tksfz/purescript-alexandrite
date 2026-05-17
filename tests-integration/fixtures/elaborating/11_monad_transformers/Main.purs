module Main where

-- Simple State monad
data State s a = State (s -> { val :: a, state :: s })

runState :: forall s a. State s a -> s -> { val :: a, state :: s }
runState = \m s -> case m of
  State f -> f s

-- Monad instance for State
bindState :: forall s a b. State s a -> (a -> State s b) -> State s b
bindState = \m f -> State (\s -> 
  let res = runState m s
  in runState (f res.val) res.state
)

pureState :: forall s a. a -> State s a
pureState = \x -> State (\s -> { val: x, state: s })

-- State operations
get :: forall s. State s s
get = State (\s -> { val: s, state: s })

put :: forall s. s -> State s {}
put = \s -> State (\_ -> { val: {}, state: s })

-- Custom bind and discard for do-notation
-- We'll use these to mock the Monad instance desugaring
bind = bindState
pure = pureState
discard = \m f -> bindState m (\_ -> f {})

test = 
  let 
    prog = do
      s <- get
      put (s + 1)
      s2 <- get
      pure (s2 * 2)
  in runState prog 10

infixl 6 add as +
infixl 7 mul as *
foreign import add :: Int -> Int -> Int
foreign import mul :: Int -> Int -> Int
